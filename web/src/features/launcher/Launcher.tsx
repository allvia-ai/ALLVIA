import { useState, useEffect, useRef, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import axios from "axios";
import {
    Zap,
    Activity,
    Terminal,
    Pin,
    Plus,
    Globe,
    Wand2,
    AppWindow,
    MessageCircle,
    Mic,
    ArrowUp,
    Circle
} from "lucide-react";
import {
    sendChatMessage,
    approveRecommendation,
    fetchRecommendations,
    fetchWorkflowProvisionOps,
    agentGoalRun,
    executeGoal,
    agentIntent,
    agentPlan,
    agentExecute,
    agentVerify,
    agentApprove,
    fetchAgentPreflight,
    runAgentPreflightFix,
    recordAgentRecoveryEvent,
    fetchTaskRuns,
    fetchTaskRun,
    fetchTaskRunStages,
    fetchTaskRunAssertions,
    fetchTaskRunArtifacts,
    fetchLockMetrics,
    fetchRuntimeInfo,
} from "@/lib/api";
import type {
    AgentPreflightCheck,
    ExecutionProfile,
    LockMetrics,
    Recommendation,
    RuntimeInfo,
    TaskRunArtifact,
    TaskRun,
    TaskStageAssertion,
    TaskStageRun,
} from "@/lib/types";
import { useRecommendations } from "@/lib/hooks";
import { emit } from "@tauri-apps/api/event"; // Added emit
import { getAllWindows, getCurrentWindow, LogicalSize } from "@tauri-apps/api/window"; // Added getAllWindows
import { invoke } from "@tauri-apps/api/core";
import ReactMarkdown, { type Components } from "react-markdown"; // Added ReactMarkdown

type LauncherResult = {
    type: "response" | "error";
    content: string;
};

type WindowWithTauriMeta = Window & {
    __TAURI_METADATA__?: unknown;
    __TAURI__?: { metadata?: unknown };
    __TAURI_INTERNALS__?: { metadata?: unknown };
};

type ApprovalContext = {
    planId: string;
    action: string;
    message: string;
    riskLevel: string;
    policy: string;
};

type RunPhase =
    | "idle"
    | "running"
    | "retrying"
    | "approval_required"
    | "manual_required"
    | "completed"
    | "failed";

type ExecutionSnapshot = {
    status: string;
    runId: string | null;
    resumeToken: string | null;
    plannerComplete: boolean;
    executionComplete: boolean;
    businessComplete: boolean;
    verifyOk: boolean;
    verifyIssues: string[];
    completionScore: {
        score: number;
        label: string;
        pass: boolean;
        reasons: string[];
    } | null;
};

type ComposerMode = "nl" | "chat" | "program";

type QuickProgramAction = {
    key: string;
    label: string;
    prompt: string;
};

type ManualResumeChecklist = {
    focusReady: boolean;
    manualStepDone: boolean;
    handsOffReady: boolean;
};

type ExecutionProfileOption = {
    value: ExecutionProfile;
    label: string;
    hint: string;
};

type StageTraceItem = {
    stage: TaskStageRun;
    assertions: TaskStageAssertion[];
    failed: TaskStageAssertion[];
};

type RecoveryAction = {
    key: string;
    label: string;
    description: string;
    kind: "preflight_fix" | "artifact" | "guided_resume";
    fixAction?: string;
    path?: string;
    assertionKey?: string;
};

type DodHistoryItem = {
    runId: string;
    createdAt: string;
    status: string;
    plannerComplete: boolean;
    executionComplete: boolean;
    businessComplete: boolean;
    assertionTotal: number;
    assertionFailed: number;
};

type DodFailureTopItem = {
    key: string;
    count: number;
    sampleActual: string;
};

type ArtifactGroupItem = {
    type: string;
    items: TaskRunArtifact[];
};

type ArtifactSortMode = "newest" | "key" | "failed_first";

type PendingDispatch = {
    prompt: string;
    executeAtMs: number;
};

const ABSOLUTE_ARTIFACT_RE = /\/(?:Users|tmp|var|private)\/[^\s"'|;,)]+/g;
const RELATIVE_ARTIFACT_RE = /\b(?:scenario_results|logs)\/[^\s"'|;,)]+/g;

const normalizeArtifactPathToken = (token: string) =>
    token.trim().replace(/[)\],.;:]+$/, "");

const extractArtifactPaths = (...inputs: Array<string | null | undefined>): string[] => {
    const unique = new Set<string>();
    for (const input of inputs) {
        if (!input) continue;
        const abs = input.match(ABSOLUTE_ARTIFACT_RE) ?? [];
        const rel = input.match(RELATIVE_ARTIFACT_RE) ?? [];
        [...abs, ...rel]
            .map(normalizeArtifactPathToken)
            .filter((x) => x.length > 0)
            .forEach((x) => unique.add(x));
    }
    return Array.from(unique).slice(0, 3);
};

const artifactPathLabel = (path: string): string => {
    const normalized = path.replace(/\\/g, "/");
    const leaf = normalized.split("/").pop() ?? normalized;
    return leaf.length > 48 ? `${leaf.slice(0, 45)}...` : leaf;
};

const normalizeDispatchPrompt = (value: string): string =>
    value.trim().replace(/\s+/g, " ").toLowerCase();

const QUICK_PROGRAM_ACTIONS: QuickProgramAction[] = [
    {
        key: "calendar_front",
        label: "캘린더 열기",
        prompt: "Calendar를 열고 전면으로 가져오세요."
    },
    {
        key: "notes_new",
        label: "새 메모",
        prompt: "Notes를 열고 새 메모를 만든 뒤 오늘 할 일 3줄을 입력하세요."
    },
    {
        key: "mail_draft",
        label: "메일 초안",
        prompt: "Mail을 열고 새 이메일 초안을 만들고 제목과 본문을 작성하세요."
    },
    {
        key: "finder_downloads",
        label: "다운로드 보기",
        prompt: "Finder를 열고 Downloads 폴더로 이동하세요."
    },
    {
        key: "scenario_1",
        label: "시나리오 1",
        prompt: "Calendar를 열고 Notes를 열어 새 메모를 작성하고 Mail로 보낼 초안을 만드세요."
    }
];

const summarizeGoalRunStatus = (params: {
    mode: ComposerMode;
    status: string;
    runId: string;
    plannerComplete: boolean;
    executionComplete: boolean;
    businessComplete: boolean;
    summary?: string | null;
}) => {
    const {
        mode,
        status,
        runId,
        plannerComplete,
        executionComplete,
        businessComplete,
        summary,
    } = params;
    const lines = [
        `**Mode**: goal-run (${mode})`,
        `**Status**: ${status}`,
        `**Run ID**: ${runId}`,
        `**Planner Complete**: ${plannerComplete ? "yes" : "no"}`,
        `**Execution Complete**: ${executionComplete ? "yes" : "no"}`,
        `**Business Complete**: ${businessComplete ? "yes" : "no"}`,
        summary ? `**Summary**: ${summary}` : "",
    ];
    return lines.filter(Boolean).join("\n");
};

const toGoalRunResultType = (status: string, businessComplete: boolean): LauncherResult["type"] => {
    const statusLower = status.toLowerCase();
    const inProgress = IN_PROGRESS_RUN_STATUSES.has(statusLower);
    if (
        businessComplete ||
        inProgress ||
        statusLower === "approval_required" ||
        statusLower === "manual_required" ||
        statusLower === "business_completed"
    ) {
        return "response";
    }
    return "error";
};

const preflightPermissionHint = (message: string): string | null => {
    const lower = message.toLowerCase();
    const likelyAutomationAuth =
        lower.includes("not authorized") ||
        lower.includes("permission denied") ||
        lower.includes("osstatus error -1002") ||
        lower.includes("(-1002)") ||
        lower.includes("osascript");
    if (!likelyAutomationAuth) return null;
    return [
        "권한 안내:",
        "- 시스템 설정 > 개인정보 보호 및 보안 > 손쉬운 사용: `AllvIa`, `Terminal` 허용",
        "- 시스템 설정 > 개인정보 보호 및 보안 > 자동화: `AllvIa`가 `Finder`/대상 앱 제어 허용",
        "- 시스템 설정 > 개인정보 보호 및 보안 > 화면 기록: `AllvIa`, `Terminal` 허용 후 앱 재시작",
    ].join("\n");
};

const QUICK_NL_SUGGESTIONS = [
    "오늘 받은 메일 5개 요약해줘",
    "노트에서 최근 TODO 정리해줘",
    "복잡 시나리오 1번 실행해줘",
    "텔레그램으로 실행 결과 요약 보내줘",
];

const QUICK_CHAT_SUGGESTIONS = [
    "안녕? 오늘 우선순위 3개만 정리해줘",
    "방금 실행 결과를 한 줄로 설명해줘",
    "지금 가장 위험한 문제 하나만 알려줘",
    "다음에 뭘 하면 좋을지 3단계로 말해줘",
];

const EXECUTION_PROFILE_OPTIONS: ExecutionProfileOption[] = [
    { value: "strict", label: "정확", hint: "충돌 시 중단" },
    { value: "test", label: "테스트", hint: "충돌 시 일시정지" },
    { value: "fast", label: "빠름", hint: "충돌 무시" },
];

const profileLabel = (profile: ExecutionProfile): string => {
    const found = EXECUTION_PROFILE_OPTIONS.find((option) => option.value === profile);
    return found?.label ?? profile;
};

const isGoalRunEndpointUnavailable = (error: unknown): boolean => {
    if (!axios.isAxiosError(error)) return false;
    const status = error.response?.status;
    if (status === 404 || status === 405 || status === 501) return true;
    const responseData = error.response?.data as { error?: unknown } | undefined;
    const text = [
        error.message ?? "",
        String(responseData ?? ""),
        String(responseData?.error ?? ""),
    ]
        .join(" ")
        .toLowerCase();
    return (
        text.includes("goal/run") ||
        text.includes("goal-run") ||
        text.includes("not found") ||
        text.includes("no route")
    );
};

const isNlChatFallbackEnabled = (): boolean => {
    if (typeof import.meta === "undefined") return false;
    // Default ON in demo builds: if NL execution path fails, degrade gracefully to chat reply.
    // Set VITE_ENABLE_NL_CHAT_FALLBACK=0 to force strict failure.
    return import.meta.env.VITE_ENABLE_NL_CHAT_FALLBACK !== "0";
};

const isLegacyGoalFallbackEnabled = (): boolean => {
    if (typeof import.meta === "undefined") return false;
    // Default OFF: keep routing on modern goal-run / intent-plan-execute path unless explicitly enabled.
    return import.meta.env.VITE_ENABLE_LEGACY_GOAL_FALLBACK === "1";
};

const shouldTryNlChatFallback = (error: unknown): boolean => {
    if (!isNlChatFallbackEnabled()) return false;
    if (!axios.isAxiosError(error)) return false;
    const status = error.response?.status;
    if (status == null) return true; // transient network issue
    if (status === 404 || status === 405 || status === 501) return true; // route unavailable
    const payload = error.response?.data as
        | { error?: unknown; message?: unknown; detail?: unknown }
        | undefined;
    const text = [
        String(payload?.error ?? ""),
        String(payload?.message ?? ""),
        String(payload?.detail ?? ""),
        error.message ?? "",
    ]
        .join(" ")
        .toLowerCase();
    return (
        (text.includes("goal/run") || text.includes("goal-run")) &&
        (text.includes("not found") || text.includes("no route"))
    );
};

const TERMINAL_RUN_STATUSES = new Set([
    "business_completed",
    "business_failed",
    "failed",
    "error",
    "blocked",
    "approval_required",
    "manual_required",
    "completed",
    "success",
]);

const IN_PROGRESS_RUN_STATUSES = new Set([
    "accepted",
    "busy",
    "queued",
    "running",
    "started",
    "retrying",
    "business_incomplete",
]);

const normalizeLoopbackTarget = (raw: string): string => {
    const trimmed = raw.trim();
    if (!trimmed) return trimmed;
    try {
        const parsed = new URL(trimmed);
        if (parsed.hostname === "127.0.0.1" || parsed.hostname === "0.0.0.0" || parsed.hostname === "::1") {
            parsed.hostname = "localhost";
        }
        return parsed.toString().replace(/\/+$/, "");
    } catch {
        return trimmed.replace(/\/+$/, "");
    }
};

const N8N_EDITOR_BASE_URL = (() => {
    if (typeof import.meta !== "undefined") {
        const raw = import.meta.env.VITE_N8N_EDITOR_URL as string | undefined;
        const trimmed = raw?.trim();
        if (trimmed) return normalizeLoopbackTarget(trimmed);
    }
    return normalizeLoopbackTarget("http://localhost:5678");
})();

const resolveRecommendationWorkflowUrl = (
    rec?: Pick<Recommendation, "workflow_url" | "workflow_id"> | null,
    workflowIdFallback?: string | null
): string | null => {
    const explicitUrl = rec?.workflow_url?.trim();
    if (explicitUrl) return normalizeLoopbackTarget(explicitUrl);
    const workflowId = rec?.workflow_id?.trim() || workflowIdFallback?.trim();
    if (!workflowId) return null;
    if (workflowId.startsWith("provisioning:")) return null;
    return `${N8N_EDITOR_BASE_URL}/workflow/${encodeURIComponent(workflowId)}`;
};

const APPROVAL_MONITOR_MAX_ATTEMPTS = 200;
const APPROVAL_MONITOR_INTERVAL_MS = 1800;
const APPROVAL_MONITOR_PENDING_NOTICE_ATTEMPT = 10;
const CHAT_TRANSCRIPT_MAX_ITEMS = 14;

type ProvisioningUiState = {
    phase: "provisioning" | "failed";
    opId: number | null;
    detail?: string;
    updatedAt: number;
};

const formatRecommendationStatusLabel = (
    rec: Recommendation,
    uiState?: ProvisioningUiState
): string => {
    if (uiState?.phase === "provisioning") {
        return "Provisioning (생성 중...)";
    }
    if (uiState?.phase === "failed") {
        return "Failed (재시도)";
    }
    if (rec.status === "failed") {
        return "Failed (재시도)";
    }
    if (rec.workflow_id?.startsWith("provisioning:")) {
        return "Provisioning (생성 중...)";
    }
    return rec.status;
};

const recommendationStatusToneClass = (
    rec: Recommendation,
    uiState?: ProvisioningUiState
): string => {
    const label = formatRecommendationStatusLabel(rec, uiState).toLowerCase();
    if (label.includes("provisioning")) {
        return "border-sky-400/40 bg-sky-500/15 text-sky-200";
    }
    if (label.includes("failed")) {
        return "border-rose-400/40 bg-rose-500/15 text-rose-200";
    }
    if (label.includes("approved") || label.includes("success")) {
        return "border-emerald-400/40 bg-emerald-500/15 text-emerald-200";
    }
    return "border-white/20 bg-white/5 text-gray-300";
};

const formatProvisionUpdatedAt = (updatedAt?: number): string => {
    if (!updatedAt) return "";
    const deltaMs = Date.now() - updatedAt;
    if (deltaMs < 10_000) return "just now";
    if (deltaMs < 60_000) return `${Math.floor(deltaMs / 1000)}s ago`;
    if (deltaMs < 3_600_000) return `${Math.floor(deltaMs / 60_000)}m ago`;
    return `${Math.floor(deltaMs / 3_600_000)}h ago`;
};

const appendChatTranscript = (
    prev: LauncherResult[],
    prompt: string,
    reply: string,
    isError: boolean
): LauncherResult[] => {
    const next: LauncherResult[] = [
        ...prev,
        { type: "response", content: `**🙋 요청**\n${prompt}` },
        {
            type: isError ? "error" : "response",
            content: `${isError ? "**⚠️ 오류**" : "**🤖 답변**"}\n${reply}`,
        },
    ];
    return next.slice(-CHAT_TRANSCRIPT_MAX_ITEMS);
};

const classifyCoreBinary = (binaryPath?: string | null): "bundle" | "workspace" | "custom" | "unknown" => {
    const normalized = binaryPath?.trim() ?? "";
    if (!normalized) return "unknown";
    if (
        normalized.includes("/Applications/AllvIa.app/Contents/MacOS/core") ||
        normalized.includes("/Applications/Steer OS.app/Contents/MacOS/core")
    ) return "bundle";
    if (normalized.includes("/local-os-agent/")) return "workspace";
    return "custom";
};

const markdownComponents: Components = {
    code({ children, ...props }) {
        const inline = 'inline' in props && props.inline;
        return !inline ? (
            <div className="bg-black/50 p-2 rounded-md my-2 overflow-x-auto font-mono text-xs border border-white/10">
                <code {...props}>{children}</code>
            </div>
        ) : (
            <code className="bg-white/10 px-1 py-0.5 rounded font-mono text-xs" {...props}>
                {children}
            </code>
        );
    },
};

export default function Launcher() {
    const [input, setInput] = useState("");
    const [isComposing, setIsComposing] = useState(false);
    const composingSinceRef = useRef<number>(0);
    const [composerMode, setComposerMode] = useState<ComposerMode>("nl");
    const [executionProfile, setExecutionProfile] =
        useState<ExecutionProfile>("strict");
    const [activeExecutionProfile, setActiveExecutionProfile] =
        useState<ExecutionProfile>("strict");
    const [autoApplyRecommendedProfile, setAutoApplyRecommendedProfile] =
        useState<boolean>(() => {
            if (typeof window === "undefined") return true;
            const raw = window.localStorage.getItem("steer.auto_profile_apply");
            return raw == null ? true : raw === "1";
        });
    const [safeExecutionMode, setSafeExecutionMode] = useState<boolean>(() => {
        if (typeof window === "undefined") return true;
        const raw = window.localStorage.getItem("steer.safe_execution_mode");
        return raw == null ? true : raw === "1";
    });
    const [compactLayoutMode, setCompactLayoutMode] = useState<boolean>(() => {
        if (typeof window === "undefined") return true;
        const raw = window.localStorage.getItem("steer.compact_layout_mode");
        return raw == null ? true : raw === "1";
    });
    const [showAdvancedControls, setShowAdvancedControls] = useState<boolean>(false);
    const [showDetailPanel, setShowDetailPanel] = useState(false);
    const [results, setResults] = useState<LauncherResult[]>([]);
    const [loading, setLoading] = useState(false);
    const [selectedIndex, setSelectedIndex] = useState(0);
    const [successPulse, setSuccessPulse] = useState(false);
    const [shake, setShake] = useState(false);
    const [approvingIds, setApprovingIds] = useState<Set<number>>(new Set());
    const [approveErrors, setApproveErrors] = useState<Record<number, string>>({});
    const [provisioningUiByRecId, setProvisioningUiByRecId] =
        useState<Record<number, ProvisioningUiState>>({});
    const [approveCooldowns, setApproveCooldowns] = useState<Record<number, number>>({});
    const [watchRecommendationIds, setWatchRecommendationIds] = useState<Set<number>>(new Set());
    const [watchRecommendationCache, setWatchRecommendationCache] = useState<Record<number, Recommendation>>({});
    const [pendingApproval, setPendingApproval] = useState<ApprovalContext | null>(null);
    const [approvalBusy, setApprovalBusy] = useState(false);
    const [lastPlanId, setLastPlanId] = useState<string | null>(null);
    const [lastStatus, setLastStatus] = useState<string | null>(null);
    const [runPhase, setRunPhase] = useState<RunPhase>("idle");
    const [dispatchBlockedReason, setDispatchBlockedReason] = useState<string | null>(null);
    const [dispatchBlockedUntilMs, setDispatchBlockedUntilMs] = useState<number | null>(null);
    const [dispatchNowMs, setDispatchNowMs] = useState<number>(Date.now());
    const [pendingDispatch, setPendingDispatch] = useState<PendingDispatch | null>(null);
    const [goalRunAvailable, setGoalRunAvailable] = useState<boolean | null>(null);
    const [runSnapshot, setRunSnapshot] = useState<ExecutionSnapshot | null>(null);
    const [stageRuns, setStageRuns] = useState<TaskStageRun[]>([]);
    const [stageAssertions, setStageAssertions] = useState<TaskStageAssertion[]>([]);
    const [taskRunArtifacts, setTaskRunArtifacts] = useState<TaskRunArtifact[]>([]);
    const [artifactTypeFilter, setArtifactTypeFilter] = useState<string>("all");
    const [artifactFailedOnly, setArtifactFailedOnly] = useState<boolean>(false);
    const [artifactSearchQuery, setArtifactSearchQuery] = useState<string>("");
    const [artifactSortMode, setArtifactSortMode] =
        useState<ArtifactSortMode>("failed_first");
    const [pinnedArtifactKeys, setPinnedArtifactKeys] = useState<Set<string>>(() => {
        if (typeof window === "undefined") return new Set<string>();
        const raw = window.localStorage.getItem("steer.artifact_pins");
        if (!raw) return new Set<string>();
        try {
            const parsed = JSON.parse(raw) as string[];
            if (!Array.isArray(parsed)) return new Set<string>();
            return new Set(parsed.filter((x) => typeof x === "string"));
        } catch {
            return new Set<string>();
        }
    });
    const [dodHistory, setDodHistory] = useState<DodHistoryItem[]>([]);
    const [dodFailureTop, setDodFailureTop] = useState<DodFailureTopItem[]>([]);
    const [dodHistoryLoading, setDodHistoryLoading] = useState(false);
    const [preflightChecks, setPreflightChecks] = useState<AgentPreflightCheck[]>([]);
    const [preflightOk, setPreflightOk] = useState<boolean | null>(null);
    const [preflightLoading, setPreflightLoading] = useState(false);
    const [preflightError, setPreflightError] = useState<string | null>(null);
    const [preflightCheckedAt, setPreflightCheckedAt] = useState<string | null>(null);
    const [preflightActiveApp, setPreflightActiveApp] = useState<string | null>(null);
    const [showPreflightDetail, setShowPreflightDetail] = useState(false);
    const [preflightFixBusy, setPreflightFixBusy] = useState<string | null>(null);
    const [preflightFixMessage, setPreflightFixMessage] = useState<string | null>(null);
    const [showDiagnostics, setShowDiagnostics] = useState(false);
    const [lockMetrics, setLockMetrics] = useState<LockMetrics | null>(null);
    const [lockMetricsError, setLockMetricsError] = useState<string | null>(null);
    const [runtimeInfo, setRuntimeInfo] = useState<RuntimeInfo | null>(null);
    const [runtimeInfoError, setRuntimeInfoError] = useState<string | null>(null);
    const [artifactOpenBusy, setArtifactOpenBusy] = useState<string | null>(null);
    const [artifactActionMessage, setArtifactActionMessage] = useState<string | null>(null);
    const [n8nOpenBusyKey, setN8nOpenBusyKey] = useState<string | null>(null);
    const [recoveryActionBusyKey, setRecoveryActionBusyKey] = useState<string | null>(null);
    const [manualChecklist, setManualChecklist] = useState<ManualResumeChecklist>({
        focusReady: false,
        manualStepDone: false,
        handsOffReady: false,
    });
    const runPollTokenRef = useRef(0);
    const approvalMonitorIdsRef = useRef<Set<number>>(new Set());
    const lastDispatchRef = useRef<{ promptKey: string; ts: number } | null>(null);
    const prevComposerModeRef = useRef<ComposerMode>("nl");
    const sendThrottleRef = useRef<number>(0);
    const inputRef = useRef<HTMLInputElement>(null);
    const scrollRef = useRef<HTMLDivElement>(null);
    const { data: recs, refetch } = useRecommendations();

    // Auto-focus input on mount
    useEffect(() => {
        inputRef.current?.focus();
    }, []);

    useEffect(() => {
        return () => {
            runPollTokenRef.current += 1;
        };
    }, []);

    useEffect(() => {
        if (composerMode === "chat" && prevComposerModeRef.current !== "chat") {
            setResults([]);
            setShowDetailPanel(true);
        }
        prevComposerModeRef.current = composerMode;
    }, [composerMode]);

    useEffect(() => {
        if (typeof window === "undefined") return;
        window.localStorage.setItem(
            "steer.auto_profile_apply",
            autoApplyRecommendedProfile ? "1" : "0"
        );
    }, [autoApplyRecommendedProfile]);

    useEffect(() => {
        if (typeof window === "undefined") return;
        window.localStorage.setItem(
            "steer.safe_execution_mode",
            safeExecutionMode ? "1" : "0"
        );
    }, [safeExecutionMode]);

    useEffect(() => {
        if (typeof window === "undefined") return;
        window.localStorage.setItem(
            "steer.compact_layout_mode",
            compactLayoutMode ? "1" : "0"
        );
    }, [compactLayoutMode]);

    useEffect(() => {
        if (typeof window === "undefined") return;
        window.localStorage.setItem(
            "steer.launcher_show_advanced",
            showAdvancedControls ? "1" : "0"
        );
    }, [showAdvancedControls]);

    useEffect(() => {
        if (typeof window === "undefined") return;
        window.localStorage.setItem(
            "steer.artifact_pins",
            JSON.stringify(Array.from(pinnedArtifactKeys))
        );
    }, [pinnedArtifactKeys]);

    useEffect(() => {
        if (!safeExecutionMode) return;
        if (executionProfile !== "strict") {
            setExecutionProfile("strict");
        }
        if (!autoApplyRecommendedProfile) {
            setAutoApplyRecommendedProfile(true);
        }
    }, [safeExecutionMode, executionProfile, autoApplyRecommendedProfile]);

    useEffect(() => {
        if (!dispatchBlockedUntilMs) return;
        const timer = window.setInterval(() => {
            setDispatchNowMs(Date.now());
        }, 1000);
        return () => window.clearInterval(timer);
    }, [dispatchBlockedUntilMs]);

    useEffect(() => {
        if (!dispatchBlockedUntilMs) return;
        if (Date.now() >= dispatchBlockedUntilMs) {
            setDispatchBlockedUntilMs(null);
            setDispatchBlockedReason(null);
        }
    }, [dispatchBlockedUntilMs, dispatchNowMs]);

    // [Phase 5.1] Visual Triggers
    const triggerSuccess = () => {
        setSuccessPulse(true);
        setTimeout(() => setSuccessPulse(false), 1000);
    };

    const triggerError = () => {
        setShake(true);
        setTimeout(() => setShake(false), 500);
    };

    // [Phase 5.2] Handle Pin
    const handlePin = async (content: string, title?: string) => {
        try {
            // 1. Emit data to 'widget' window
            await emit('pin-data', {
                type: 'text',
                content,
                title: title || 'Pinned from AllvIa'
            });

            // 2. Show 'widget' window if hidden
            const windows = await getAllWindows();
            const widgetWin = windows.find(w => w.label === 'widget');
            if (widgetWin) {
                await widgetWin.show();
            }

            triggerSuccess();
        } catch (error) {
            console.error("Pin failed", error);
            triggerError();
        }
    };

    // Combine all navigable items
    const pendingRecs = recs?.filter(r => r.status === 'pending') ?? [];
    const suggestionRecs = (() => {
        const merged = new Map<number, Recommendation>();
        pendingRecs.forEach((rec) => merged.set(rec.id, rec));
        watchRecommendationIds.forEach((id) => {
            const live = recs?.find((rec) => rec.id === id);
            if (live) {
                merged.set(id, live);
                return;
            }
            const cached = watchRecommendationCache[id];
            if (cached) {
                merged.set(id, cached);
            }
        });
        return Array.from(merged.values());
    })();
    const coreBinaryKind = classifyCoreBinary(runtimeInfo?.binary_path);
    const isDevBundleMismatch =
        typeof import.meta !== "undefined" &&
        Boolean(import.meta.env.DEV) &&
        coreBinaryKind === "bundle";
    const failedAssertions = stageAssertions.filter(a => !a.passed);
    const compactEvidence = (raw?: string | null) => {
        if (!raw) return "";
        return raw.length > 140 ? `${raw.slice(0, 140)}...` : raw;
    };
    const compactMetadata = (raw?: string | null) => {
        if (!raw) return "";
        const text = raw.trim();
        if (!text) return "";
        try {
            const parsed = JSON.parse(text) as unknown;
            const normalized = JSON.stringify(parsed);
            return normalized.length > 160
                ? `${normalized.slice(0, 160)}...`
                : normalized;
        } catch {
            return text.length > 160 ? `${text.slice(0, 160)}...` : text;
        }
    };
    const stageTraceItems: StageTraceItem[] = stageRuns
        .slice()
        .sort((a, b) => a.stage_order - b.stage_order)
        .map((stage) => {
            const assertions = stageAssertions.filter((a) => a.stage_name === stage.stage_name);
            const failed = assertions.filter((a) => !a.passed);
            return { stage, assertions, failed };
        });
    const recoveryAssertions = stageAssertions
        .filter((a) => a.stage_name === "recovery")
        .slice(-10)
        .reverse();
    const failedArtifactKeys = new Set(
        failedAssertions.map((item) => item.assertion_key)
    );
    const artifactTypeOptions = [
        "all",
        ...Array.from(new Set(taskRunArtifacts.map((item) => item.artifact_type))).sort(
            (a, b) => a.localeCompare(b)
        ),
    ];
    const artifactSearchLower = artifactSearchQuery.trim().toLowerCase();
    const artifactGroups: ArtifactGroupItem[] = (() => {
        if (taskRunArtifacts.length === 0) return [];
        const artifacts = taskRunArtifacts
            .filter((item) => {
                if (
                    artifactTypeFilter !== "all" &&
                    item.artifact_type !== artifactTypeFilter
                ) {
                    return false;
                }
                if (artifactFailedOnly && !failedArtifactKeys.has(item.artifact_key)) {
                    return false;
                }
                if (!artifactSearchLower) return true;
                const haystack = [
                    item.artifact_type,
                    item.artifact_key,
                    item.value,
                    item.metadata ?? "",
                ]
                    .join("\n")
                    .toLowerCase();
                return haystack.includes(artifactSearchLower);
            })
            .sort((a, b) => {
                const aPinned = pinnedArtifactKeys.has(a.artifact_key) ? 1 : 0;
                const bPinned = pinnedArtifactKeys.has(b.artifact_key) ? 1 : 0;
                if (aPinned !== bPinned) return bPinned - aPinned;
                if (artifactSortMode === "key") {
                    return a.artifact_key.localeCompare(b.artifact_key);
                }
                if (artifactSortMode === "failed_first") {
                    const aFailed = failedArtifactKeys.has(a.artifact_key) ? 1 : 0;
                    const bFailed = failedArtifactKeys.has(b.artifact_key) ? 1 : 0;
                    if (aFailed !== bFailed) return bFailed - aFailed;
                }
                return (
                    new Date(b.created_at).getTime() - new Date(a.created_at).getTime()
                );
            });
        const bucket = new Map<string, TaskRunArtifact[]>();
        for (const item of artifacts) {
            const group = item.artifact_type?.trim() || "unknown";
            const prev = bucket.get(group) ?? [];
            prev.push(item);
            bucket.set(group, prev);
        }
        return Array.from(bucket.entries())
            .sort((a, b) => a[0].localeCompare(b[0]))
            .map(([type, items]) => ({ type, items }));
    })();
    const focusPreflight = preflightChecks.find((c) => c.key === "focus_handoff");
    const accessibilityPreflight = preflightChecks.find((c) => c.key === "accessibility");
    const screenCapturePreflight = preflightChecks.find((c) => c.key === "screen_capture");
    const focusPreflightBlocked = !!focusPreflight && !focusPreflight.ok;
    const focusActualLower = (focusPreflight?.actual ?? "").toLowerCase();
    const focusNeedsHandoff =
        !!focusPreflight &&
        focusPreflight.ok &&
        focusActualLower.length > 0 &&
        !focusActualLower.includes("finder") &&
        !focusActualLower.includes("skipped");
    const profileRecommendation = (() => {
        if (preflightOk === false) {
            return {
                profile: "strict" as ExecutionProfile,
                reason: "점검 실패 상태에서는 정확(Strict)로만 복구를 권장합니다.",
            };
        }
        if (
            focusPreflight &&
            focusPreflight.ok &&
            (focusPreflight.actual ?? "").toLowerCase().includes("skipped")
        ) {
            return {
                profile: "test" as ExecutionProfile,
                reason: "포커스 점검이 비활성화되어 테스트 프로필을 권장합니다.",
            };
        }
        if (focusPreflightBlocked || focusNeedsHandoff) {
            return {
                profile: "test" as ExecutionProfile,
                reason: "포커스 전환 불안정으로 테스트 프로필을 권장합니다.",
            };
        }
        return {
            profile: "strict" as ExecutionProfile,
            reason: "권한/포커스가 안정적이므로 정확(Strict)을 권장합니다.",
        };
    })();
    const firstFailedArtifactPath = (() => {
        for (const assertion of failedAssertions) {
            const artifacts = extractArtifactPaths(
                assertion.evidence,
                `${assertion.actual} ${assertion.expected}`
            );
            if (artifacts.length > 0) {
                return artifacts[0];
            }
        }
        return null;
    })();
    const recoveryActions = (() => {
        const actions: RecoveryAction[] = [];
        const seen = new Set<string>();
        const failureBlob = failedAssertions
            .map((a) => `${a.assertion_key} ${a.evidence ?? ""} ${a.actual}`)
            .join("\n")
            .toLowerCase();
        const hasFailureHint = (hints: string[]) =>
            hints.some((hint) => failureBlob.includes(hint));
        const push = (action: RecoveryAction) => {
            if (seen.has(action.key)) return;
            seen.add(action.key);
            actions.push(action);
        };

        if (preflightOk === false) {
            if (accessibilityPreflight && !accessibilityPreflight.ok) {
                push({
                    key: "fix-accessibility",
                    label: "접근성 설정 열기",
                    description: "접근성 권한 허용 후 다시 점검",
                    kind: "preflight_fix",
                    fixAction: "open_accessibility_settings",
                    assertionKey: "recovery.preflight.open_accessibility_settings",
                });
            }
            if (screenCapturePreflight && !screenCapturePreflight.ok) {
                push({
                    key: "fix-screen-capture",
                    label: "화면 기록 설정 열기",
                    description: "화면/오디오 녹화 권한 허용 후 다시 점검",
                    kind: "preflight_fix",
                    fixAction: "open_screen_capture_settings",
                    assertionKey: "recovery.preflight.open_screen_capture_settings",
                });
            }
        }

        if (focusPreflightBlocked || focusNeedsHandoff) {
            push({
                key: "fix-focus",
                label: "Finder 전면 복구",
                description: "포커스 충돌 복구",
                kind: "preflight_fix",
                fixAction: "activate_finder",
                assertionKey: "recovery.preflight.activate_finder",
            });
            push({
                key: "fix-isolated",
                label: "격리 모드 준비",
                description: "다른 앱 숨김 후 실행 안정화",
                kind: "preflight_fix",
                fixAction: "prepare_isolated_mode",
                assertionKey: "recovery.preflight.prepare_isolated_mode",
            });
        }

        if (
            hasFailureHint([
                "artifact.mail_recipient_present",
                "contract_missing_mail_recipient",
                "mail_recipient=",
                "ambiguous_draft",
                "mail_send_proof",
                "outgoing=",
            ])
        ) {
            push({
                key: "fix-mail-recipient",
                label: "메일 수신자 보강",
                description: "run prompt/기본 수신자로 받는 사람 자동 채우기",
                kind: "preflight_fix",
                fixAction: "mail_fill_default_recipient",
                assertionKey: "recovery.preflight.mail_fill_default_recipient",
            });
            push({
                key: "fix-activate-mail",
                label: "Mail 전면 전환",
                description: "메일 작성창 포커스 복구",
                kind: "preflight_fix",
                fixAction: "activate_mail",
                assertionKey: "recovery.preflight.activate_mail",
            });
            push({
                key: "fix-mail-outgoing-cleanup",
                label: "Mail 초안창 정리",
                description: "누적된 outgoing 창 숨김 처리",
                kind: "preflight_fix",
                fixAction: "mail_cleanup_outgoing_windows",
                assertionKey: "recovery.preflight.mail_cleanup_outgoing_windows",
            });
        }

        if (
            hasFailureHint([
                "artifact.notes_write_confirmed",
                "artifact.notes_note_id_present",
                "contract_missing_notes",
            ])
        ) {
            push({
                key: "fix-activate-notes",
                label: "Notes 전면 전환",
                description: "노트 작성 타깃 포커스 복구",
                kind: "preflight_fix",
                fixAction: "activate_notes",
                assertionKey: "recovery.preflight.activate_notes",
            });
        }

        if (
            hasFailureHint([
                "artifact.textedit_write_confirmed",
                "artifact.textedit_doc_id_present",
                "artifact.textedit_save_confirmed",
                "contract_missing_textedit",
                "contract_textedit_body_empty",
            ])
        ) {
            push({
                key: "fix-activate-textedit",
                label: "TextEdit 전면 전환",
                description: "TextEdit 작성 대상 포커스 복구",
                kind: "preflight_fix",
                fixAction: "activate_textedit",
                assertionKey: "recovery.preflight.activate_textedit",
            });
            push({
                key: "fix-textedit-save",
                label: "TextEdit 저장 실행",
                description: "front document 저장(Cmd+S 대체)",
                kind: "preflight_fix",
                fixAction: "textedit_save_front_document",
                assertionKey: "recovery.preflight.textedit_save_front_document",
            });
        }

        if (firstFailedArtifactPath) {
            push({
                key: `artifact:${firstFailedArtifactPath}`,
                label: `증거 열기 (${artifactPathLabel(firstFailedArtifactPath)})`,
                description: "실패 근거 파일 열기",
                kind: "artifact",
                path: firstFailedArtifactPath,
                assertionKey: "recovery.artifact.open",
            });
        }

        if (lastStatus === "manual_required" && lastPlanId && !pendingApproval) {
            push({
                key: "guided-resume",
                label: "자동 복구 + Resume",
                description: "증거 확인 후 즉시 재개",
                kind: "guided_resume",
                assertionKey: "recovery.guided_resume",
            });
        }

        return actions.slice(0, 4);
    })();

    const runScore = runSnapshot
        ? (() => {
              if (runSnapshot.completionScore) {
                  return {
                      score: runSnapshot.completionScore.score,
                      label: runSnapshot.completionScore.label,
                      pass: runSnapshot.completionScore.pass,
                  };
              }
              let score = 0;
              if (runSnapshot.plannerComplete) score += 15;
              if (runSnapshot.executionComplete) score += 20;
              if (runSnapshot.businessComplete) score += 30;
              if (runSnapshot.verifyOk) score += 20;
              if (runSnapshot.status === "completed" || runSnapshot.status === "success") score += 15;
              score -= Math.min(10, failedAssertions.length * 2);
              score = Math.max(0, Math.min(100, score));
              const label = score >= 90 ? "Excellent" : score >= 75 ? "Good" : score >= 60 ? "Needs tuning" : "Risky";
              return { score, label, pass: score >= 75 };
          })()
        : null;

    const recoveryActionForFailureKey = (failureKey: string): RecoveryAction | null => {
        const key = failureKey.toLowerCase();
        if (key.includes("mail_recipient")) {
            return {
                key: `topfix:${failureKey}:mail_fill_default_recipient`,
                label: "수신자 보강",
                description: "Mail 받는 사람 자동 보강",
                kind: "preflight_fix",
                fixAction: "mail_fill_default_recipient",
                assertionKey: "recovery.preflight.mail_fill_default_recipient",
            };
        }
        if (key.includes("ambiguous_draft") || key.includes("mail_send_proof") || key.includes("outgoing")) {
            return {
                key: `topfix:${failureKey}:mail_cleanup_outgoing_windows`,
                label: "Mail 초안창 정리",
                description: "누적 outgoing 창 숨김 처리",
                kind: "preflight_fix",
                fixAction: "mail_cleanup_outgoing_windows",
                assertionKey: "recovery.preflight.mail_cleanup_outgoing_windows",
            };
        }
        if (key.includes("textedit_save")) {
            return {
                key: `topfix:${failureKey}:textedit_save`,
                label: "TextEdit 저장",
                description: "TextEdit front document 저장 실행",
                kind: "preflight_fix",
                fixAction: "textedit_save_front_document",
                assertionKey: "recovery.preflight.textedit_save_front_document",
            };
        }
        if (key.includes("textedit")) {
            return {
                key: `topfix:${failureKey}:activate_textedit`,
                label: "TextEdit 전면",
                description: "TextEdit 포커스 복구",
                kind: "preflight_fix",
                fixAction: "activate_textedit",
                assertionKey: "recovery.preflight.activate_textedit",
            };
        }
        if (key.includes("notes")) {
            return {
                key: `topfix:${failureKey}:activate_notes`,
                label: "Notes 전면",
                description: "Notes 포커스 복구",
                kind: "preflight_fix",
                fixAction: "activate_notes",
                assertionKey: "recovery.preflight.activate_notes",
            };
        }
        if (key.includes("focus")) {
            return {
                key: `topfix:${failureKey}:activate_finder`,
                label: "Finder 전면",
                description: "Focus handoff 복구",
                kind: "preflight_fix",
                fixAction: "activate_finder",
                assertionKey: "recovery.preflight.activate_finder",
            };
        }
        return null;
    };

    const nextActionHint = (() => {
        if (preflightOk === false) {
            if (focusPreflightBlocked) {
                return "Finder 전면 복구 또는 격리 모드 준비 후 다시 점검하세요.";
            }
            if (accessibilityPreflight && !accessibilityPreflight.ok) {
                return "접근성 설정에서 Codex/Terminal을 허용한 뒤 다시 점검하세요.";
            }
            if (screenCapturePreflight && !screenCapturePreflight.ok) {
                return "화면 기록 설정에서 Codex/Terminal 허용 후 다시 점검하세요.";
            }
            return "실행 전 점검 통과가 필요합니다.";
        }
        if (safeExecutionMode) {
            return "안전 모드 ON: Strict 프로필 고정으로 실행합니다.";
        }
        if (executionProfile !== profileRecommendation.profile) {
            return `권장 프로필은 ${profileLabel(profileRecommendation.profile)}입니다. (${profileRecommendation.reason})`;
        }
        if (pendingApproval) {
            return "승인 버튼(once/always/deny)으로 다음 단계를 선택하세요.";
        }
        if (lastStatus === "manual_required" && lastPlanId) {
            return "체크리스트 3개를 확인한 뒤 Resume으로 다음 단계를 진행하세요.";
        }
        if (runSnapshot?.completionScore && !runSnapshot.completionScore.pass) {
            return `완성도 미달: ${runSnapshot.completionScore.reasons[0] ?? "증거 부족"} 보완 후 재실행하세요.`;
        }
        if (runPhase === "failed") {
            if (failedAssertions.length > 0) {
                const first = failedAssertions[0];
                return `실패 근거: ${first.stage_name}.${first.assertion_key} (expected=${first.expected}, actual=${first.actual})`;
            }
            return "실패 로그를 확인하고 조건을 보강해 재실행하세요.";
        }
        if (runPhase === "completed") {
            return "완료되었습니다. 결과를 확인하고 다음 요청을 진행하세요.";
        }
        if (runPhase === "running" || runPhase === "retrying") {
            return "실행 중입니다. 입력 충돌을 피하고 기다려주세요.";
        }
        return "자연어 요청 또는 프로그램 버튼으로 실행을 시작하세요.";
    })();

    const updateExecutionState = (snapshot: ExecutionSnapshot) => {
        setRunSnapshot(snapshot);
        const statusLower = snapshot.status.toLowerCase();
        if (statusLower === "approval_required") {
            setRunPhase("approval_required");
            return;
        }
        if (statusLower === "manual_required") {
            setRunPhase("manual_required");
            return;
        }
        if (IN_PROGRESS_RUN_STATUSES.has(statusLower)) {
            setRunPhase("running");
            return;
        }
        if (["failed", "error", "blocked"].includes(statusLower)) {
            setRunPhase("failed");
            return;
        }
        if (statusLower === "business_completed") {
            setRunPhase("completed");
            return;
        }
        if ((statusLower === "completed" || statusLower === "success") && snapshot.verifyOk) {
            setRunPhase("completed");
            return;
        }
        if (snapshot.businessComplete && snapshot.verifyOk) {
            setRunPhase("completed");
            return;
        }
        setRunPhase("failed");
    };

    const hudMeta: Record<RunPhase, { label: string; chip: string; dot: string }> = {
        idle: {
            label: "엔진 대기 중",
            chip: "bg-white/10 text-gray-300 border-white/15",
            dot: "bg-gray-400",
        },
        running: {
            label: "실행 중",
            chip: "bg-blue-500/20 text-blue-200 border-blue-400/40",
            dot: "bg-blue-400",
        },
        retrying: {
            label: "재시도 중",
            chip: "bg-amber-500/20 text-amber-200 border-amber-400/40",
            dot: "bg-amber-400",
        },
        approval_required: {
            label: "승인 필요",
            chip: "bg-rose-500/20 text-rose-200 border-rose-400/40",
            dot: "bg-rose-400",
        },
        manual_required: {
            label: "수동 단계 필요",
            chip: "bg-sky-500/20 text-sky-200 border-sky-400/40",
            dot: "bg-sky-400",
        },
        completed: {
            label: "완료",
            chip: "bg-emerald-500/20 text-emerald-200 border-emerald-400/40",
            dot: "bg-emerald-400",
        },
        failed: {
            label: "실패",
            chip: "bg-rose-500/20 text-rose-200 border-rose-400/40",
            dot: "bg-rose-400",
        },
    };
    const currentHud = hudMeta[runPhase];

    const dodItems = runSnapshot
        ? [
              {
                  key: "planner",
                  label: "Planner 완료",
                  done: runSnapshot.plannerComplete,
                  detail: runSnapshot.plannerComplete ? "done" : "not done",
              },
              {
                  key: "execution",
                  label: "Execution 완료",
                  done: runSnapshot.executionComplete,
                  detail: runSnapshot.executionComplete ? "done" : "not done",
              },
              {
                  key: "business",
                  label: "Business 완료",
                  done: runSnapshot.businessComplete,
                  detail: runSnapshot.businessComplete ? "done" : "not done",
              },
              {
                  key: "verify",
                  label: "의미/검증 통과",
                  done: runSnapshot.verifyOk,
                  detail: runSnapshot.verifyOk
                      ? "ok"
                      : `${runSnapshot.verifyIssues.length} issue(s)`,
              },
              {
                  key: "final",
                  label: "최종 상태 성공",
                  done: runSnapshot.status === "completed" || runSnapshot.status === "success",
                  detail: runSnapshot.status,
              },
          ]
        : [];

    const hasDetailContent =
        results.length > 0 ||
        suggestionRecs.length > 0 ||
        !!pendingApproval ||
        (lastStatus === "manual_required" && !!lastPlanId) ||
        stageRuns.length > 0 ||
        stageAssertions.length > 0 ||
        taskRunArtifacts.length > 0 ||
        !!runSnapshot;
    const isChatComposerMode = composerMode === "chat";
    const shouldShowDetailPanel = isChatComposerMode ? true : showDetailPanel;
    const shouldRenderDetailPanel =
        shouldShowDetailPanel && (isChatComposerMode || hasDetailContent);
    const checkpointHoldActive =
        runPhase === "manual_required" || runPhase === "approval_required";
    const isExecutionLocked =
        loading ||
        approvalBusy ||
        runPhase === "running" ||
        runPhase === "retrying" ||
        (safeExecutionMode && checkpointHoldActive) ||
        !!pendingDispatch;
    const dispatchRetrySeconds = dispatchBlockedUntilMs
        ? Math.max(0, Math.ceil((dispatchBlockedUntilMs - dispatchNowMs) / 1000))
        : 0;
    const safeCountdownSeconds = pendingDispatch
        ? Math.max(0, Math.ceil((pendingDispatch.executeAtMs - dispatchNowMs) / 1000))
        : 0;
    const executionLockHint = (() => {
        if (dispatchBlockedReason) {
            if (dispatchRetrySeconds > 0) {
                return `${dispatchBlockedReason} · ${dispatchRetrySeconds}초 후 재시도`;
            }
            return dispatchBlockedReason;
        }
        if (pendingDispatch) {
            return `안전 모드 카운트다운 ${safeCountdownSeconds}초`;
        }
        if (!isExecutionLocked) return null;
        if (loading) return "요청 실행 준비 중입니다.";
        if (approvalBusy) return "승인 처리 중입니다.";
        if (runPhase === "running") return "실행 중입니다. 현재 run이 끝나면 새 요청을 보낼 수 있습니다.";
        if (runPhase === "retrying") return "재시도 중입니다. 완료 후 다시 요청하세요.";
        if (safeExecutionMode && runPhase === "manual_required")
            return "안전 모드: 수동 단계 완료 전에는 새 요청을 잠급니다. 아래 Resume을 진행하세요.";
        if (safeExecutionMode && runPhase === "approval_required")
            return "안전 모드: 승인 체크포인트 해결 전에는 새 요청을 잠급니다.";
        return "현재 실행 잠금 상태입니다.";
    })();
    const showPreflightPanel =
        showAdvancedControls ||
        preflightLoading ||
        preflightOk === false ||
        !!preflightError;
    const navigableItems = [
        ...results.map((r, i) => ({ type: 'result', data: r, id: `res-${i}` })),
        ...suggestionRecs.map(r => ({ type: 'recommendation', data: r, id: `rec-${r.id}` }))
    ];

    const setProvisioningUiState = useCallback(
        (
            id: number,
            phase: ProvisioningUiState["phase"],
            options?: { opId?: number | null; detail?: string }
        ) => {
            setProvisioningUiByRecId((prev) => ({
                ...prev,
                [id]: {
                    phase,
                    opId: options?.opId ?? prev[id]?.opId ?? null,
                    detail: options?.detail,
                    updatedAt: Date.now(),
                },
            }));
        },
        []
    );

    const clearProvisioningUiState = useCallback((id: number) => {
        setProvisioningUiByRecId((prev) => {
            if (!(id in prev)) return prev;
            const next = { ...prev };
            delete next[id];
            return next;
        });
    }, []);

    const addWatchRecommendation = useCallback((id: number, fallback?: Recommendation | null) => {
        setWatchRecommendationIds((prev) => {
            const next = new Set(prev);
            next.add(id);
            return next;
        });
        if (fallback) {
            setWatchRecommendationCache((prev) => ({ ...prev, [id]: fallback }));
        }
    }, []);

    const removeWatchRecommendation = useCallback((id: number) => {
        setWatchRecommendationIds((prev) => {
            if (!prev.has(id)) return prev;
            const next = new Set(prev);
            next.delete(id);
            return next;
        });
        setWatchRecommendationCache((prev) => {
            if (!(id in prev)) return prev;
            const next = { ...prev };
            delete next[id];
            return next;
        });
        setProvisioningUiByRecId((prev) => {
            if (!(id in prev)) return prev;
            const next = { ...prev };
            delete next[id];
            return next;
        });
    }, []);

    const loadRunDiagnostics = async (runId?: string | null) => {
        if (!runId) {
            setStageRuns([]);
            setStageAssertions([]);
            setTaskRunArtifacts([]);
            return;
        }
        try {
            const [stages, assertions, artifacts] = await Promise.all([
                fetchTaskRunStages(runId),
                fetchTaskRunAssertions(runId),
                fetchTaskRunArtifacts(runId),
            ]);
            setStageRuns(stages);
            setStageAssertions(assertions);
            setTaskRunArtifacts(artifacts);
        } catch (error) {
            console.error("Failed to load stage diagnostics", error);
            setStageRuns([]);
            setStageAssertions([]);
            setTaskRunArtifacts([]);
        }
    };

    const loadLockMetrics = useCallback(async () => {
        try {
            const metrics = await fetchLockMetrics();
            setLockMetrics(metrics);
            setLockMetricsError(null);
        } catch (error) {
            console.error("Failed to load lock metrics", error);
            setLockMetrics(null);
            setLockMetricsError("lock metrics unavailable");
        }
    }, []);

    const loadRuntimeInfo = useCallback(async () => {
        try {
            const info = await fetchRuntimeInfo();
            setRuntimeInfo(info);
            setRuntimeInfoError(null);
        } catch (error) {
            console.error("Failed to load runtime info", error);
            setRuntimeInfo(null);
            setRuntimeInfoError("runtime info unavailable");
        }
    }, []);

    useEffect(() => {
        void loadRuntimeInfo();
    }, [loadRuntimeInfo]);

    useEffect(() => {
        if (!showDiagnostics) return;
        void loadLockMetrics();
        void loadRuntimeInfo();
    }, [showDiagnostics, loadLockMetrics, loadRuntimeInfo]);

    const loadDodHistory = useCallback(async () => {
        setDodHistoryLoading(true);
        try {
            const runs = await fetchTaskRuns(6);
            const sorted = runs
                .slice()
                .sort((a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime())
                .slice(0, 5);
            const failureCounter = new Map<string, { count: number; sampleActual: string }>();
            const history = await Promise.all(
                sorted.map(async (run) => {
                    try {
                        const assertions = await fetchTaskRunAssertions(run.run_id);
                        for (const assertion of assertions) {
                            if (assertion.passed) continue;
                            const key = assertion.assertion_key;
                            const prev = failureCounter.get(key);
                            if (prev) {
                                prev.count += 1;
                            } else {
                                failureCounter.set(key, {
                                    count: 1,
                                    sampleActual: assertion.actual,
                                });
                            }
                        }
                        return {
                            runId: run.run_id,
                            createdAt: run.created_at,
                            status: run.status,
                            plannerComplete: run.planner_complete,
                            executionComplete: run.execution_complete,
                            businessComplete: run.business_complete,
                            assertionTotal: assertions.length,
                            assertionFailed: assertions.filter((a) => !a.passed).length,
                        } satisfies DodHistoryItem;
                    } catch {
                        return {
                            runId: run.run_id,
                            createdAt: run.created_at,
                            status: run.status,
                            plannerComplete: run.planner_complete,
                            executionComplete: run.execution_complete,
                            businessComplete: run.business_complete,
                            assertionTotal: 0,
                            assertionFailed: 0,
                        } satisfies DodHistoryItem;
                    }
                })
            );
            setDodHistory(history);
            const topFailures = Array.from(failureCounter.entries())
                .sort((a, b) => b[1].count - a[1].count)
                .slice(0, 3)
                .map(([key, value]) => ({
                    key,
                    count: value.count,
                    sampleActual: value.sampleActual,
                }));
            setDodFailureTop(topFailures);
        } catch (error) {
            console.error("Failed to load DoD history", error);
            setDodHistory([]);
            setDodFailureTop([]);
        } finally {
            setDodHistoryLoading(false);
        }
    }, []);

    const toSnapshotFromTaskRun = (run: TaskRun): ExecutionSnapshot => {
        const statusLower = run.status.toLowerCase();
        const verifyOk = run.business_complete || statusLower === "business_completed";
        return {
            status: run.status,
            runId: run.run_id,
            resumeToken: null,
            plannerComplete: !!run.planner_complete,
            executionComplete: !!run.execution_complete,
            businessComplete: !!run.business_complete,
            verifyOk,
            verifyIssues: verifyOk ? [] : [`status=${run.status}`],
            completionScore: null,
        };
    };

    const pollRunStatusUntilTerminal = useCallback(
        async (runId: string, mode: ComposerMode = "nl") => {
            if (!runId) return;
            const token = Date.now();
            runPollTokenRef.current = token;

            for (let i = 0; i < 60; i += 1) {
                if (runPollTokenRef.current !== token) return;
                try {
                    const run = await fetchTaskRun(runId);
                    if (runPollTokenRef.current !== token) return;
                    setLastStatus(run.status);
                    updateExecutionState(toSnapshotFromTaskRun(run));

                    if (i % 2 === 0) {
                        await loadRunDiagnostics(runId);
                    }

                    const statusLower = run.status.toLowerCase();
                    if (TERMINAL_RUN_STATUSES.has(statusLower)) {
                        await loadRunDiagnostics(runId);
                        await loadDodHistory();
                        const terminalSummary = summarizeGoalRunStatus({
                            mode,
                            status: run.status,
                            runId: run.run_id,
                            plannerComplete: !!run.planner_complete,
                            executionComplete: !!run.execution_complete,
                            businessComplete: !!run.business_complete,
                            summary: statusLower === "business_completed"
                                ? "goal completed and business checks passed"
                                : undefined,
                        });
                        setResults([
                            {
                                type: toGoalRunResultType(run.status, !!run.business_complete),
                                content: terminalSummary,
                            },
                        ]);
                        if (statusLower === "business_completed") {
                            triggerSuccess();
                        } else if (!["approval_required", "manual_required"].includes(statusLower)) {
                            triggerError();
                        }
                        return;
                    }
                } catch (error) {
                    // transient read failure while run is still being updated
                    console.warn("run polling failed", error);
                }
                await new Promise((resolve) => window.setTimeout(resolve, 1500));
            }
        },
        [
            loadDodHistory,
            loadRunDiagnostics,
            updateExecutionState,
            triggerError,
            triggerSuccess,
        ]
    );

    const runPreflightCheck = useCallback(async (silent: boolean = false): Promise<boolean> => {
        setPreflightLoading(true);
        setPreflightError(null);
        if (!silent) {
            setPreflightFixMessage(null);
        }
        try {
            const preflight = await fetchAgentPreflight();
            setPreflightChecks(preflight.checks);
            setPreflightOk(preflight.ok);
            setPreflightCheckedAt(preflight.checked_at);
            setPreflightActiveApp(preflight.active_app ?? null);
            if (!preflight.ok) {
                setShowPreflightDetail(true);
                if (!silent) {
                    setResults([
                        {
                            type: "error",
                            content:
                                "실행 전 점검 실패로 실행을 차단했습니다.\n- Accessibility/Screen/Focus 상태를 확인한 뒤 다시 실행하세요.\n- Focus Handoff가 실패하면 전용 데스크톱(또는 다른 사용자 세션)에서 실행하세요.",
                        },
                    ]);
                }
            } else if (!silent) {
                setShowPreflightDetail(false);
            }
            return preflight.ok;
        } catch (error) {
            if (axios.isAxiosError(error) && error.response?.status === 404) {
                const nowIso = new Date().toISOString();
                setPreflightChecks([
                    {
                        key: "legacy_core_preflight",
                        label: "Preflight API",
                        ok: true,
                        expected: "/api/agent/preflight",
                        actual: "legacy_core_mode",
                        message:
                            "Legacy core detected (preflight API unavailable). Proceeding without preflight gate.",
                    },
                ]);
                setPreflightOk(true);
                setPreflightCheckedAt(nowIso);
                setPreflightActiveApp(null);
                setPreflightError(null);
                if (!silent) {
                    setResults([
                        {
                            type: "response",
                            content:
                                "⚠️ 실행 전 점검 API가 없는 코어 버전입니다. 이번 실행은 preflight 게이트 없이 진행합니다.",
                        },
                    ]);
                }
                return true;
            }
            const message = error instanceof Error ? error.message : String(error);
            setPreflightOk(false);
            setPreflightError(message);
            setShowPreflightDetail(true);
            if (!silent) {
                setResults([
                    {
                        type: "error",
                        content: `실행 전 점검 API 호출 실패: ${message}`,
                    },
                ]);
            }
            return false;
        } finally {
            setPreflightLoading(false);
        }
    }, []);

    const handlePreflightFix = useCallback(async (action: string) => {
        setPreflightFixBusy(action);
        setPreflightFixMessage(null);
        try {
            const currentRunId = runSnapshot?.runId ?? null;
            const assertionKey = `recovery.preflight.${action}`;
            const fix = await runAgentPreflightFix(action, {
                run_id: currentRunId,
                stage_name: "recovery",
                assertion_key: assertionKey,
            });
            const front = fix.active_app ? ` (front=${fix.active_app})` : "";
            const recordedMessage =
                fix.recorded && fix.run_id
                    ? ` · 기록됨(run=${fix.run_id})`
                    : currentRunId
                      ? " · 기록 없음(run 미존재)"
                      : "";
            const hint = preflightPermissionHint(fix.message);
            setPreflightFixMessage(
                hint
                    ? `${fix.message}${front}${recordedMessage}\n${hint}`
                    : `${fix.message}${front}${recordedMessage}`
            );
            if (fix.recorded && fix.run_id) {
                await loadRunDiagnostics(fix.run_id);
                await loadDodHistory();
            }
            await runPreflightCheck(true);
        } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            const hint = preflightPermissionHint(message);
            setPreflightFixMessage(
                hint ? `자동 조치 실패: ${message}\n${hint}` : `자동 조치 실패: ${message}`
            );
        } finally {
            setPreflightFixBusy(null);
        }
    }, [loadDodHistory, loadRunDiagnostics, runPreflightCheck, runSnapshot?.runId]);

    const recordRecoveryAction = useCallback(
        async (payload: {
            actionKey: string;
            status: "completed" | "failed";
            details: string;
            runId?: string | null;
            stageName?: string;
            expected?: string;
            actual?: string;
        }) => {
            const runId = payload.runId ?? runSnapshot?.runId ?? dodHistory[0]?.runId ?? null;
            if (!runId) return;
            try {
                const rec = await recordAgentRecoveryEvent({
                    run_id: runId,
                    action_key: payload.actionKey,
                    status: payload.status,
                    details: payload.details,
                    stage_name: payload.stageName ?? "recovery",
                    expected: payload.expected ?? "completed",
                    actual: payload.actual ?? payload.status,
                });
                if (rec.recorded) {
                    await loadRunDiagnostics(rec.run_id);
                    await loadDodHistory();
                }
            } catch (error) {
                const message = error instanceof Error ? error.message : String(error);
                setArtifactActionMessage(`복구 기록 실패: ${message}`);
            }
        },
        [dodHistory, loadDodHistory, loadRunDiagnostics, runSnapshot?.runId]
    );

    const openArtifactPath = useCallback(async (path: string) => {
        const candidate = path.trim();
        if (!candidate) return;
        setArtifactOpenBusy(candidate);
        setArtifactActionMessage(null);
        let actionStatus: "completed" | "failed" = "completed";
        let actionDetails = `artifact=${candidate}`;
        try {
            const tauriMeta =
                (window as WindowWithTauriMeta).__TAURI_METADATA__ ||
                (window as WindowWithTauriMeta).__TAURI__?.metadata ||
                (window as WindowWithTauriMeta).__TAURI_INTERNALS__?.metadata;
            if (tauriMeta) {
                const opened = await invoke<string>("open_artifact_path", { path: candidate });
                actionDetails = `artifact opened: ${opened}`;
                setArtifactActionMessage(`증거 열기 완료: ${opened}`);
            } else if (navigator?.clipboard?.writeText) {
                await navigator.clipboard.writeText(candidate);
                actionDetails = `artifact copied to clipboard: ${candidate}`;
                setArtifactActionMessage(`웹 모드: 경로를 클립보드에 복사했습니다 (${candidate})`);
            } else {
                actionDetails = `artifact manual open requested: ${candidate}`;
                setArtifactActionMessage(`웹 모드: 경로를 수동으로 열어주세요 (${candidate})`);
            }
        } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            actionStatus = "failed";
            actionDetails = `artifact open failed: ${message}`;
            setArtifactActionMessage(`증거 열기 실패: ${message}`);
        } finally {
            await recordRecoveryAction({
                actionKey: "recovery.artifact.open",
                status: actionStatus,
                details: actionDetails,
                actual: actionStatus,
            });
            setArtifactOpenBusy(null);
        }
    }, [recordRecoveryAction]);

    const openExternalTarget = useCallback(async (target: string) => {
        const candidate = target.trim();
        if (!candidate) {
            throw new Error("empty target");
        }
        const tauriMeta =
            (window as WindowWithTauriMeta).__TAURI_METADATA__ ||
            (window as WindowWithTauriMeta).__TAURI__?.metadata ||
            (window as WindowWithTauriMeta).__TAURI_INTERNALS__?.metadata;
        if (tauriMeta) {
            await invoke<string>("open_external_target", { target: candidate });
            return;
        }
        const popup = window.open(candidate, "_blank", "noopener,noreferrer");
        if (!popup) {
            throw new Error("popup_blocked");
        }
    }, []);

    const monitorApprovedWorkflow = useCallback(
        async (
            id: number,
            fallback?: Recommendation | null,
            initialProvisionOpId?: number | null
        ) => {
            if (approvalMonitorIdsRef.current.has(id)) return;
            approvalMonitorIdsRef.current.add(id);
            addWatchRecommendation(id, fallback ?? null);
            let trackedProvisionOpId = initialProvisionOpId ?? null;
            let pendingNoticeShown = false;
            const reportProvisionFailure = (reason: string) => {
                setProvisioningUiState(id, "failed", {
                    opId: trackedProvisionOpId,
                    detail: reason,
                });
                setApproveErrors((prev) => ({
                    ...prev,
                    [id]: `워크플로 생성 실패: ${reason}`,
                }));
                setResults((prev) => [
                    ...prev.slice(-6),
                    {
                        type: "error",
                        content: [
                            "**Workflow 생성 실패**",
                            `- recommendation_id: \`${id}\``,
                            trackedProvisionOpId != null
                                ? `- provision_op_id: \`${trackedProvisionOpId}\``
                                : "",
                            `- 상세: ${reason}`,
                            `- 수동 확인: ${N8N_EDITOR_BASE_URL}`,
                            "- Retry를 누르면 다시 생성을 시도합니다.",
                        ]
                            .filter(Boolean)
                            .join("\n"),
                    },
                ]);
                setShowDetailPanel(true);
            };
            try {
                for (let attempt = 0; attempt < APPROVAL_MONITOR_MAX_ATTEMPTS; attempt += 1) {
                    const latest = await fetchRecommendations();
                    const current = latest.find((rec) => rec.id === id) ?? null;
                    if (current) {
                        setWatchRecommendationCache((prev) => ({ ...prev, [id]: current }));
                    }
                    const currentStatus = current?.status?.toLowerCase() ?? "";
                    if (currentStatus === "failed" || currentStatus === "rejected") {
                        const reason = current?.last_error?.trim() || "workflow provisioning failed";
                        reportProvisionFailure(reason);
                        removeWatchRecommendation(id);
                        return;
                    }

                    let latestProvisionOp: Awaited<
                        ReturnType<typeof fetchWorkflowProvisionOps>
                    >[number] | null = null;
                    try {
                        const ops = await fetchWorkflowProvisionOps({
                            recommendationId: id,
                            limit: 10,
                        });
                        if (trackedProvisionOpId != null) {
                            latestProvisionOp =
                                ops.find((op) => op.id === trackedProvisionOpId) ?? ops[0] ?? null;
                        } else {
                            latestProvisionOp = ops[0] ?? null;
                        }
                        if (latestProvisionOp) {
                            trackedProvisionOpId = latestProvisionOp.id;
                        }
                    } catch {
                        // Ignore transient provision-op API failures and continue fallback polling.
                    }

                    const opStatus = latestProvisionOp?.status?.toLowerCase() ?? "";
                    if (opStatus === "failed" || opStatus === "reconcile_needed") {
                        const reason =
                            latestProvisionOp?.error?.trim() ||
                            current?.last_error?.trim() ||
                            "workflow provisioning failed";
                        reportProvisionFailure(reason);
                        removeWatchRecommendation(id);
                        return;
                    }

                    const workflowIdFromOp = latestProvisionOp?.workflow_id?.trim() || null;
                    const workflowUrl =
                        resolveRecommendationWorkflowUrl(current, workflowIdFromOp) ||
                        resolveRecommendationWorkflowUrl(fallback ?? null, workflowIdFromOp);
                    const workflowId =
                        workflowIdFromOp || current?.workflow_id?.trim() || fallback?.workflow_id?.trim() || null;

                    if (workflowUrl) {
                        await openExternalTarget(workflowUrl);
                        clearProvisioningUiState(id);
                        setResults([
                            {
                                type: "response",
                                content: [
                                    "**Workflow 승인 완료**",
                                    `- recommendation_id: \`${id}\``,
                                    workflowId ? `- workflow_id: \`${workflowId}\`` : "",
                                    trackedProvisionOpId != null
                                        ? `- provision_op_id: \`${trackedProvisionOpId}\``
                                        : "",
                                    `- n8n 편집기: ${workflowUrl}`,
                                    "- 워크플로를 자동으로 열었습니다.",
                                ]
                                    .filter(Boolean)
                                    .join("\n"),
                            },
                        ]);
                        setShowDetailPanel(true);
                        removeWatchRecommendation(id);
                        triggerSuccess();
                        return;
                    }

                    if (
                        (opStatus === "requested" || opStatus === "created") &&
                        attempt >= APPROVAL_MONITOR_PENDING_NOTICE_ATTEMPT &&
                        !pendingNoticeShown
                    ) {
                        setProvisioningUiState(id, "provisioning", {
                            opId: trackedProvisionOpId,
                            detail: opStatus || "requested",
                        });
                        pendingNoticeShown = true;
                        setResults([
                            {
                                type: "response",
                                content: [
                                    "**승인 완료 · 생성 대기**",
                                    `- recommendation_id: \`${id}\``,
                                    trackedProvisionOpId != null
                                        ? `- provision_op_id: \`${trackedProvisionOpId}\``
                                        : "",
                                    "- n8n 워크플로 생성 요청은 접수됐지만 아직 완료되지 않았습니다.",
                                    "- 잠시 후 자동 재시도하거나, n8n 상태를 먼저 확인하세요.",
                                ]
                                    .filter(Boolean)
                                    .join("\n"),
                            },
                        ]);
                    } else if (opStatus === "requested" || opStatus === "created") {
                        setProvisioningUiState(id, "provisioning", {
                            opId: trackedProvisionOpId,
                            detail: opStatus,
                        });
                    }

                    await new Promise((resolve) =>
                        window.setTimeout(resolve, APPROVAL_MONITOR_INTERVAL_MS)
                    );
                }
                reportProvisionFailure("workflow URL generation timeout");
            } catch (error) {
                const message = error instanceof Error ? error.message : String(error);
                reportProvisionFailure(`workflow monitor failed: ${message}`);
            } finally {
                approvalMonitorIdsRef.current.delete(id);
                void refetch();
            }
        },
        [
            addWatchRecommendation,
            clearProvisioningUiState,
            openExternalTarget,
            refetch,
            removeWatchRecommendation,
            setProvisioningUiState,
        ]
    );

    const copyTextValue = useCallback(async (value: string, label: string) => {
        const candidate = value.trim();
        if (!candidate) return;
        try {
            if (navigator?.clipboard?.writeText) {
                await navigator.clipboard.writeText(candidate);
                setArtifactActionMessage(`${label} 복사 완료: ${candidate}`);
                return;
            }
            const el = document.createElement("textarea");
            el.value = candidate;
            el.style.position = "fixed";
            el.style.left = "-9999px";
            document.body.appendChild(el);
            el.focus();
            el.select();
            document.execCommand("copy");
            document.body.removeChild(el);
            setArtifactActionMessage(`${label} 복사 완료: ${candidate}`);
        } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            setArtifactActionMessage(`${label} 복사 실패: ${message}`);
        }
    }, []);

    const copyArtifactPayload = useCallback(async (artifact: TaskRunArtifact) => {
        const payload = JSON.stringify(
            {
                run_id: artifact.run_id,
                artifact_type: artifact.artifact_type,
                artifact_key: artifact.artifact_key,
                value: artifact.value,
                metadata: artifact.metadata ?? null,
                created_at: artifact.created_at,
            },
            null,
            2
        );
        try {
            if (navigator?.clipboard?.writeText) {
                await navigator.clipboard.writeText(payload);
                setArtifactActionMessage(`복사 완료: ${artifact.artifact_key}`);
                return;
            }
            const el = document.createElement("textarea");
            el.value = payload;
            el.style.position = "fixed";
            el.style.left = "-9999px";
            document.body.appendChild(el);
            el.focus();
            el.select();
            document.execCommand("copy");
            document.body.removeChild(el);
            setArtifactActionMessage(`복사 완료: ${artifact.artifact_key}`);
        } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            setArtifactActionMessage(`복사 실패: ${message}`);
        }
    }, []);

    const togglePinArtifactKey = useCallback((artifactKey: string) => {
        setPinnedArtifactKeys((prev) => {
            const next = new Set(prev);
            if (next.has(artifactKey)) {
                next.delete(artifactKey);
            } else {
                next.add(artifactKey);
            }
            return next;
        });
    }, []);

    useEffect(() => {
        if (composerMode === "chat") {
            if (runPhase === "completed" || runPhase === "running") {
                setShowDetailPanel(true);
            }
            return;
        }
        if (
            pendingApproval ||
            lastStatus === "manual_required" ||
            runPhase === "failed" ||
            failedAssertions.length > 0
        ) {
            setShowDetailPanel(true);
            return;
        }
        if (
            runPhase === "completed" &&
            failedAssertions.length === 0 &&
            !pendingApproval
        ) {
            setShowDetailPanel(false);
        }
    }, [pendingApproval, lastStatus, runPhase, failedAssertions.length, composerMode]);

    useEffect(() => {
        if (lastStatus === "manual_required") {
            setManualChecklist({
                focusReady: false,
                manualStepDone: false,
                handsOffReady: false,
            });
        }
    }, [lastStatus, lastPlanId]);

    useEffect(() => {
        void runPreflightCheck(true);
    }, [runPreflightCheck]);

    useEffect(() => {
        void loadDodHistory();
    }, [loadDodHistory]);

    useEffect(() => {
        if (!pendingDispatch) return;
        if (dispatchNowMs < pendingDispatch.executeAtMs) return;
        const prompt = pendingDispatch.prompt;
        setPendingDispatch(null);
        setDispatchBlockedReason(null);
        setDispatchBlockedUntilMs(null);
        void dispatchPrompt(prompt, true);
    }, [pendingDispatch, dispatchNowMs]);

    useEffect(() => {
        if (failedAssertions.length > 0 || preflightOk === false) {
            setShowDiagnostics(true);
        }
    }, [failedAssertions.length, preflightOk]);

    useEffect(() => {
        const tauriMeta =
            (window as WindowWithTauriMeta).__TAURI_METADATA__ ||
            (window as WindowWithTauriMeta).__TAURI__?.metadata ||
            (window as WindowWithTauriMeta).__TAURI_INTERNALS__?.metadata;
        if (!tauriMeta) {
            return;
        }

        const showOperationalPanels = composerMode !== "chat";
        const isExpanded = shouldRenderDetailPanel;
        const preflightExpanded =
            showOperationalPanels &&
            showPreflightPanel &&
            (showPreflightDetail ||
                showAdvancedControls ||
                preflightOk === false ||
                !!preflightError ||
                !!executionLockHint);
        const desiredWidth = compactLayoutMode ? 1080 : 1240;
        const maxViewportWidth =
            typeof window !== "undefined"
                ? Math.max(960, (window.screen?.availWidth ?? window.innerWidth) - 24)
                : desiredWidth;
        const targetWidth = Math.min(desiredWidth, maxViewportWidth);
        const targetHeight = isExpanded
            ? compactLayoutMode
                ? 640
                : 720
            : preflightExpanded
              ? compactLayoutMode
                  ? 252
                  : 292
              : compactLayoutMode
                ? 182
                : 214;

        void getCurrentWindow()
            .setSize(new LogicalSize(targetWidth, targetHeight))
            .catch((error) => {
                console.error("Failed to sync launcher window size:", error);
            });
    }, [
        showDetailPanel,
        hasDetailContent,
        shouldRenderDetailPanel,
        showPreflightPanel,
        showPreflightDetail,
        showAdvancedControls,
        preflightOk,
        preflightError,
        executionLockHint,
        compactLayoutMode,
        composerMode,
    ]);

    // Reset selection when items change
    useEffect(() => {
        setSelectedIndex(0);
    }, [results, suggestionRecs.length]);

    // Scroll to selected item
    useEffect(() => {
        if (scrollRef.current && navigableItems.length > 0) {
            const selectedElement = scrollRef.current.children[selectedIndex];
            if (selectedElement) {
                selectedElement.scrollIntoView({
                    behavior: 'smooth',
                    block: 'nearest',
                });
            }
        }
    }, [selectedIndex, navigableItems.length]);

    const dispatchPrompt = async (rawPrompt: string, bypassSafeCountdown: boolean = false) => {
        const prompt = rawPrompt.trim();
        if (!prompt) return;
        const promptKey = normalizeDispatchPrompt(prompt);
        if (
            !bypassSafeCountdown &&
            pendingDispatch &&
            normalizeDispatchPrompt(pendingDispatch.prompt) === promptKey
        ) {
            setDispatchBlockedReason("중복 실행 차단");
            setDispatchBlockedUntilMs(pendingDispatch.executeAtMs);
            setResults([
                {
                    type: "error",
                    content:
                        "이미 같은 요청이 안전 카운트다운 중입니다. 취소하거나 카운트다운 완료를 기다리세요.",
                },
            ]);
            setShowDetailPanel(true);
            return;
        }
        if (composerMode !== "chat" && safeExecutionMode && !bypassSafeCountdown) {
            const executeAtMs = Date.now() + 3000;
            setPendingDispatch({ prompt, executeAtMs });
            setDispatchBlockedReason("안전 카운트다운");
            setDispatchBlockedUntilMs(executeAtMs);
            setResults([
                {
                    type: "response",
                    content:
                        `**안전 실행 대기**\n- ${Math.max(
                            1,
                            Math.ceil((executeAtMs - Date.now()) / 1000)
                        )}초 후 자동 실행됩니다.\n- 취소 버튼으로 중단할 수 있습니다.`,
                },
            ]);
            setShowDetailPanel(true);
            setRunPhase("idle");
            return;
        }
        if (composerMode !== "chat" && bypassSafeCountdown && pendingDispatch) {
            setPendingDispatch(null);
        }
        if (loading) {
            setDispatchBlockedReason("이전 요청 준비 중");
            setDispatchBlockedUntilMs(Date.now() + 4000);
            setResults([
                {
                    type: "error",
                    content:
                        "이전 요청을 준비 중입니다. 잠시 후 다시 실행하세요.",
                },
            ]);
            setShowDetailPanel(true);
            return;
        }
        if (isExecutionLocked) {
            setDispatchBlockedReason("실행 잠금 상태");
            setDispatchBlockedUntilMs(null);
            setResults([
                {
                    type: "error",
                    content:
                        "실행 중에는 새 요청을 받지 않습니다. 현재 실행이 끝난 뒤 다시 시도하세요.",
                },
            ]);
            setShowDetailPanel(true);
            return;
        }
        const now = Date.now();
        const duplicateWindowMs = 12000;
        if (
            lastDispatchRef.current &&
            lastDispatchRef.current.promptKey === promptKey &&
            now - lastDispatchRef.current.ts < duplicateWindowMs
        ) {
            const retryAt = lastDispatchRef.current.ts + duplicateWindowMs;
            setDispatchBlockedReason("중복 실행 차단");
            setDispatchBlockedUntilMs(retryAt);
            setResults([
                {
                    type: "error",
                    content: "중복 실행 차단: 같은 요청이 방금 실행되었습니다. 결과 패널을 확인하거나 잠시 후 다시 실행하세요.",
                },
            ]);
            setRunPhase("idle");
            setShowDetailPanel(true);
            return;
        }
        lastDispatchRef.current = { promptKey, ts: now };
        setDispatchBlockedReason(null);
        setDispatchBlockedUntilMs(null);

        if (composerMode === "chat") {
            setShowDetailPanel(true);
            setLoading(true);
            setRunPhase("running");
            setPendingApproval(null);
            setRecoveryActionBusyKey(null);

            try {
                const res = await sendChatMessage(prompt);
                const content =
                    typeof res.response === "string" && res.response.trim().length > 0
                        ? res.response
                        : "✅ 요청을 받았어요. 한 문장만 더 구체적으로 말해주면 바로 도와줄게요.";
                setResults((prev) => appendChatTranscript(prev, prompt, content, false));
                setInput("");
                setRunPhase("completed");
                triggerSuccess();
            } catch (error) {
                const message =
                    error instanceof Error ? error.message : "Failed to reach chat agent.";
                setResults((prev) => appendChatTranscript(prev, prompt, message, true));
                setRunPhase("failed");
                triggerError();
            } finally {
                setLoading(false);
            }
            return;
        }

        const preflightReady = await runPreflightCheck(false);
        if (!preflightReady) {
            setDispatchBlockedReason("Preflight 차단");
            setDispatchBlockedUntilMs(null);
            setRunPhase("failed");
            triggerError();
            setShowDetailPanel(true);
            return;
        }
        let effectiveProfile = safeExecutionMode ? "strict" : executionProfile;
        if (
            !safeExecutionMode &&
            executionProfile === "strict" &&
            profileRecommendation.profile === "test"
        ) {
            if (autoApplyRecommendedProfile) {
                effectiveProfile = profileRecommendation.profile;
                setExecutionProfile(effectiveProfile);
            } else {
                setDispatchBlockedReason("권장 프로필 불일치");
                setDispatchBlockedUntilMs(null);
                setResults([
                    {
                        type: "error",
                        content: `실행 차단: 현재 상태에서는 ${profileLabel(profileRecommendation.profile)} 프로필을 권장합니다.\n사유: ${profileRecommendation.reason}`,
                    },
                ]);
                setRunPhase("failed");
                setShowDetailPanel(true);
                triggerError();
                return;
            }
        }

        setShowDetailPanel(true);
        setLoading(true);
        setRunPhase("running");
        setActiveExecutionProfile(effectiveProfile);
        setRunSnapshot(null);
        setStageRuns([]);
        setStageAssertions([]);
        setResults([]);
        setDispatchBlockedReason(null);
        setDispatchBlockedUntilMs(null);
        setArtifactActionMessage(null);
        setPendingApproval(null);
        setRecoveryActionBusyKey(null);

        // [Phase 6.3] Performance Test Command
        if (prompt === "test_perf") {
            if (!import.meta.env.DEV) {
                setResults([
                    {
                        type: "error",
                        content: "`test_perf`는 개발 모드에서만 사용할 수 있습니다.",
                    },
                ]);
                setShowDetailPanel(true);
                setRunPhase("failed");
                triggerError();
                setLoading(false);
                return;
            }
            const start = performance.now();
            const dummyItems: LauncherResult[] = Array.from({ length: 1000 }, (_, i) => ({
                type: 'response' as const,
                content: `**Perf Item #${i + 1}**: This is a dummy item to test rendering performance. ${Math.random()}`
            }));
            const end = performance.now();
            setResults(dummyItems);
            setInput("");
            triggerSuccess();
            setLoading(false);
            console.log(`[Perf] Generated 1000 items in ${(end - start).toFixed(2)}ms`);
            return;
        }

        try {
            const useGoalRunPath =
                (composerMode === "nl" || composerMode === "program") &&
                goalRunAvailable !== false;
            let fallbackToLegacyFromGoalRun = false;
            if (useGoalRunPath) {
                try {
                    const goalRes = await agentGoalRun(prompt);
                    if (goalRunAvailable !== true) {
                        setGoalRunAvailable(true);
                    }
                    const goalStatusLower = goalRes.status.toLowerCase();
                    const goalStatusInProgress = IN_PROGRESS_RUN_STATUSES.has(goalStatusLower);
                    setLastPlanId(null);
                    setLastStatus(goalRes.status);
                    updateExecutionState({
                        status: goalRes.status,
                        runId: goalRes.run_id,
                        resumeToken: null,
                        plannerComplete: !!goalRes.planner_complete,
                        executionComplete: !!goalRes.execution_complete,
                        businessComplete: !!goalRes.business_complete,
                        verifyOk: !!goalRes.business_complete,
                        verifyIssues: goalRes.business_complete ? [] : ["business_complete=false"],
                        completionScore: null,
                    });
                    await loadRunDiagnostics(goalRes.run_id);
                    await loadDodHistory();
                    const summary = summarizeGoalRunStatus({
                        mode: composerMode,
                        status: goalRes.status,
                        runId: goalRes.run_id,
                        plannerComplete: !!goalRes.planner_complete,
                        executionComplete: !!goalRes.execution_complete,
                        businessComplete: !!goalRes.business_complete,
                        summary: goalRes.summary,
                    });
                    setResults([
                        {
                            type: toGoalRunResultType(goalRes.status, !!goalRes.business_complete),
                            content: summary,
                        },
                    ]);
                    setInput("");

                    if (goalStatusLower === "approval_required") {
                        setRunPhase("approval_required");
                    } else if (goalStatusLower === "manual_required") {
                        setRunPhase("manual_required");
                    } else if (
                        goalRes.business_complete ||
                        goalStatusLower === "business_completed"
                    ) {
                        setRunPhase("completed");
                        triggerSuccess();
                    } else if (goalStatusInProgress) {
                        setRunPhase("running");
                        void pollRunStatusUntilTerminal(goalRes.run_id, composerMode);
                    } else {
                        setRunPhase("failed");
                        triggerError();
                    }
                    return;
                } catch (goalRunError) {
                    if (!isGoalRunEndpointUnavailable(goalRunError)) {
                        throw goalRunError;
                    }
                    setGoalRunAvailable(false);
                    if (isLegacyGoalFallbackEnabled()) {
                        fallbackToLegacyFromGoalRun = true;
                        console.warn(
                            "goal-run endpoint unavailable. Falling back to legacy goal path.",
                            goalRunError
                        );
                    } else {
                        console.warn(
                            "goal-run endpoint unavailable. Legacy fallback disabled; using intent/plan/execute path.",
                            goalRunError
                        );
                    }
                }
            }

            if (fallbackToLegacyFromGoalRun) {
                try {
                    const legacyRes = await executeGoal(prompt);
                    const legacyStatus = (legacyRes.status || "started").toLowerCase();
                    setLastPlanId(null);
                    setLastStatus(legacyRes.status);
                    setRunSnapshot({
                        status: legacyRes.status || "started",
                        runId: null,
                        resumeToken: null,
                        plannerComplete: legacyStatus !== "error",
                        executionComplete: false,
                        businessComplete: false,
                        verifyOk: legacyStatus !== "error",
                        verifyIssues: legacyStatus === "error" ? [legacyRes.message] : [],
                        completionScore: null,
                    });
                    setResults([{
                        type: legacyStatus === "error" ? "error" : "response",
                        content: [
                            `**Mode**: legacy-goal (${composerMode})`,
                            `**Status**: ${legacyRes.status || "started"}`,
                            `**Message**: ${legacyRes.message || "Goal started."}`,
                        ].join("\n"),
                    }]);
                    if (legacyStatus === "error") {
                        setRunPhase("failed");
                        triggerError();
                    } else {
                        setRunPhase("running");
                        triggerSuccess();
                        setInput("");
                    }
                    return;
                } catch (legacyGoalError) {
                    console.warn(
                        "legacy goal endpoint failed. Trying intent/plan/execute fallback.",
                        legacyGoalError
                    );
                }
            }

            const intentRes = await agentIntent(prompt);
            if (intentRes.missing_slots && intentRes.missing_slots.length > 0) {
                const followUp = intentRes.follow_up || "추가 정보가 필요합니다.";
                setResults([{
                    type: 'response',
                    content: `**추가 입력 필요**\n- Missing: ${intentRes.missing_slots.join(", ")}\n- ${followUp}`
                }]);
                setRunPhase("idle");
                setLoading(false);
                return;
            }

            const planRes = await agentPlan(intentRes.session_id);
            setLastPlanId(planRes.plan_id);
            if (planRes.missing_slots?.length) {
                setResults([{
                    type: 'response',
                    content: `**추가 입력 필요**\n- Missing: ${planRes.missing_slots.join(", ")}`
                }]);
                setRunPhase("idle");
                setLoading(false);
                return;
            }
            const execRes = await agentExecute(planRes.plan_id, effectiveProfile);
            setLastStatus(execRes.status);
            const verifyRes = await agentVerify(planRes.plan_id);
            updateExecutionState({
                status: execRes.status,
                runId: execRes.run_id ?? null,
                resumeToken: execRes.resume_token ?? null,
                plannerComplete: !!execRes.planner_complete,
                executionComplete: !!execRes.execution_complete,
                businessComplete: !!execRes.business_complete,
                verifyOk: !!verifyRes.ok,
                verifyIssues: verifyRes.issues ?? [],
                completionScore: execRes.completion_score ?? null,
            });
            await loadRunDiagnostics(execRes.run_id);
            await loadDodHistory();

            const summaryLines = [
                fallbackToLegacyFromGoalRun
                    ? `**Mode**: legacy fallback (${composerMode})`
                    : "",
                `**Intent**: ${intentRes.intent} (${Math.round(intentRes.confidence * 100)}%)`,
                `**Status**: ${execRes.status}`,
                `**Profile**: ${execRes.profile ?? effectiveProfile}${execRes.collision_policy ? ` (collision=${execRes.collision_policy})` : ""}`,
                execRes.run_id ? `**Run ID**: ${execRes.run_id}` : "",
                execRes.resume_token ? `**Resume Token**: ${execRes.resume_token}` : "",
                `**Planner Complete**: ${execRes.planner_complete ? "yes" : "no"}`,
                `**Execution Complete**: ${execRes.execution_complete ? "yes" : "no"}`,
                `**Business Complete**: ${execRes.business_complete ? "yes" : "no"}`,
                `**Verify**: ${verifyRes.ok ? "ok" : "issues"}`,
                execRes.completion_score
                    ? `**Completion Score**: ${execRes.completion_score.score} (${execRes.completion_score.label})`
                    : "",
                execRes.resume_from != null ? `**Next Step**: ${execRes.resume_from + 1}` : "",
            ];
            const dodChecks = execRes.stage_dod ?? [];
            const dodFailed = dodChecks.filter((item) => !item.passed);
            if (dodChecks.length > 0) {
                summaryLines.push(`**DoD Checks**: ${dodChecks.length - dodFailed.length}/${dodChecks.length} passed`);
            }
            const logLines = execRes.logs?.slice(0, 10).map(line => `- ${line}`) ?? [];
            const verifyLines = verifyRes.issues?.length ? verifyRes.issues.map(i => `- ${i}`) : [];
            const manualLines = execRes.manual_steps?.length
                ? execRes.manual_steps.map(step => `- ${step}`)
                : [];
            const dodLines = dodChecks
                .slice(0, 12)
                .map((item) => `- ${item.passed ? "✅" : "❌"} [${item.stage}] ${item.key} (expected=${item.expected}, actual=${item.actual})`);
            const dodFailLines = dodFailed
                .slice(0, 8)
                .map((item) => `- [${item.stage}] ${item.key} (${item.actual})`);

            if (execRes.status === "approval_required" && execRes.approval?.action) {
                setPendingApproval({
                    planId: planRes.plan_id,
                    action: execRes.approval.action,
                    message: execRes.approval.message,
                    riskLevel: execRes.approval.risk_level,
                    policy: execRes.approval.policy,
                });
            }

            setResults([{
                type: 'response',
                content: [
                    summaryLines.filter(Boolean).join("\n"),
                    logLines.length ? `\n**Logs**\n${logLines.join("\n")}` : "",
                    verifyLines.length ? `\n**Verify Issues**\n${verifyLines.join("\n")}` : "",
                    dodLines.length ? `\n**Stage DoD**\n${dodLines.join("\n")}` : "",
                    dodFailLines.length ? `\n**DoD Failed**\n${dodFailLines.join("\n")}` : "",
                    manualLines.length ? `\n**Manual Steps**\n${manualLines.join("\n")}` : "",
                    execRes.status === "approval_required" && execRes.approval
                        ? `\n**Approval Required**\n- Action: ${execRes.approval.action}\n- Risk: ${execRes.approval.risk_level}\n- Policy: ${execRes.approval.policy}\n- ${execRes.approval.message}`
                        : ""
                ].join("\n")
            }]);
            setInput("");
            triggerSuccess();
        } catch (error) {
            console.error("Launcher send failed", error);
            const maybe = error as {
                message?: string;
                response?: {
                    status?: number;
                    data?: {
                        error?: string;
                        message?: string;
                        detail?: string;
                        lock_scope?: string;
                        active_plan_id?: string;
                    };
                };
            };
            const apiErr = maybe.response?.data?.error;
            if (
                typeof apiErr === "string" &&
                [
                    "agent_execution_in_progress_global",
                    "plan_execution_in_progress",
                    "plan_execution_in_progress_db",
                ].includes(apiErr)
            ) {
                const scope = maybe.response?.data?.lock_scope ?? "plan";
                const activePlan = maybe.response?.data?.active_plan_id ?? "";
                const detail =
                    typeof maybe.response?.data?.message === "string"
                        ? maybe.response?.data?.message
                        : "다른 실행이 진행 중입니다.";
                setResults([
                    {
                        type: "error",
                        content: [
                            "**실행 충돌 감지**",
                            `- scope: ${scope}`,
                            activePlan ? `- active_plan_id: ${activePlan}` : "",
                            `- ${detail}`,
                        ]
                            .filter(Boolean)
                            .join("\n"),
                    },
                ]);
                setRunPhase("failed");
                triggerError();
                return;
            }
            if (shouldTryNlChatFallback(error)) {
                try {
                    const res = await sendChatMessage(prompt);
                    const content =
                        typeof res.response === "string" && res.response.trim().length > 0
                            ? res.response
                            : "✅ 요청을 받았어요. 한 문장만 더 구체적으로 말해주면 바로 도와줄게요.";
                    setResults([{ type: "response", content }]);
                    setInput("");
                    setRunPhase("completed");
                    triggerSuccess();
                    return;
                } catch {
                    // fall through to explicit error below
                }
            }
            const statusCode = maybe.response?.status;
            const errorDetail =
                maybe.response?.data?.detail &&
                typeof maybe.response.data.detail === "string"
                    ? maybe.response.data.detail
                    : "";
            const lowerErr = `${maybe.response?.data?.error ?? ""} ${errorDetail} ${maybe.message ?? ""}`.toLowerCase();
            const detailMsg =
                errorDetail ||
                maybe.response?.data?.message ||
                maybe.response?.data?.error ||
                maybe.message ||
                "Failed to reach agent.";
            const normalizedMsg =
                lowerErr.includes("screen capture unavailable") ||
                lowerErr.includes("permission missing")
                    ? "화면 캡처 권한이 없어 실행이 중단됐습니다. 시스템 설정에서 AllvIa/Terminal의 화면 기록 권한을 켠 뒤 다시 시도하세요."
                    : detailMsg;
            setResults([
                {
                    type: "error",
                    content: `실행 실패${statusCode ? ` (${statusCode})` : ""}: ${normalizedMsg}`,
                },
            ]);
            setRunPhase("failed");
            triggerError();
        } finally {
            setLoading(false);
        }
    };

    const cancelPendingDispatch = useCallback(() => {
        if (!pendingDispatch) return;
        setPendingDispatch(null);
        setDispatchBlockedReason("안전 카운트다운 취소");
        setDispatchBlockedUntilMs(null);
        setResults([
            {
                type: "response",
                content: "**안전 실행 취소됨**\n- 자동 실행을 취소했습니다.",
            },
        ]);
        setShowDetailPanel(true);
    }, [pendingDispatch]);

    const handleSend = async () => {
        const prompt = input.trim();
        if (!prompt || loading || isExecutionLocked) {
            return;
        }
        // IME composition state가 드물게 고착되는 케이스를 방어한다.
        if (isComposing) {
            const composingMs = Date.now() - (composingSinceRef.current || Date.now());
            if (composingMs < 1500) {
                return;
            }
            setIsComposing(false);
            composingSinceRef.current = 0;
        }
        if (isComposing) {
            return;
        }
        const now = Date.now();
        if (now - sendThrottleRef.current < 650) {
            return;
        }
        sendThrottleRef.current = now;
        await dispatchPrompt(prompt);
    };

    const handleSuggestionClick = (suggestion: string) => {
        setIsComposing(false);
        setInput(suggestion);
        inputRef.current?.focus();
    };

    const handleQuickProgramAction = async (action: QuickProgramAction) => {
        await dispatchPrompt(action.prompt);
    };

    const handleTelegramListenerCommand = async (
        command: "telegram listener start" | "telegram listener status"
    ) => {
        if (loading || isExecutionLocked) return;
        setShowDetailPanel(true);
        setLoading(true);
        setRunPhase("running");
        setPendingApproval(null);
        setRecoveryActionBusyKey(null);
        try {
            const res = await sendChatMessage(command);
            setResults([
                {
                    type: "response",
                    content: [
                        "**Telegram Listener**",
                        `- 요청: \`${command}\``,
                        `- 응답: ${res.response}`,
                    ].join("\n"),
                },
            ]);
            setRunPhase("completed");
            triggerSuccess();
        } catch (error) {
            const message =
                error instanceof Error ? error.message : "Telegram listener command failed.";
            setResults([{ type: "error", content: message }]);
            setRunPhase("failed");
            triggerError();
        } finally {
            setLoading(false);
        }
    };

    const extractErrorMessage = (error: unknown) => {
        if (typeof error === "string") return error;
        if (error && typeof error === "object") {
            const maybe = error as {
                message?: unknown;
                response?: { data?: { error?: unknown } };
            };
            const responseError = maybe.response?.data?.error;
            if (typeof responseError === "string") return responseError;
            if (typeof maybe.message === "string") return maybe.message;
        }
        return "Approve failed";
    };

    const resumeExecution = async (skipChecklist = false) => {
        if (!lastPlanId) return;
        const checklistReady =
            skipChecklist ||
            (manualChecklist.focusReady &&
                manualChecklist.manualStepDone &&
                manualChecklist.handsOffReady);
        if (!checklistReady) {
            setResults([
                {
                    type: "error",
                    content:
                        "Resume 차단: 체크리스트 3개(포커스 복구/수동 단계 완료/입력 충돌 방지)를 모두 확인해야 합니다.",
                },
            ]);
            setShowDetailPanel(true);
            triggerError();
            return;
        }
        if (skipChecklist) {
            setManualChecklist({
                focusReady: true,
                manualStepDone: true,
                handsOffReady: true,
            });
        }
        const preflightReady = await runPreflightCheck(true);
        if (!preflightReady) {
            setResults([
                {
                    type: "error",
                    content:
                        "Resume 차단: preflight가 통과되지 않았습니다. 포커스/권한 상태를 복구한 뒤 다시 시도하세요.",
                },
            ]);
            setRunPhase("manual_required");
            setShowDetailPanel(true);
            triggerError();
            return;
        }
        setLoading(true);
        setRunPhase("retrying");
        try {
            const resumeProfile = safeExecutionMode ? "strict" : activeExecutionProfile;
            const execRes = await agentExecute(lastPlanId, resumeProfile, {
                resumeToken: runSnapshot?.resumeToken ?? null,
            });
            setLastStatus(execRes.status);
            const verifyRes = await agentVerify(lastPlanId);
            updateExecutionState({
                status: execRes.status,
                runId: execRes.run_id ?? null,
                resumeToken: execRes.resume_token ?? null,
                plannerComplete: !!execRes.planner_complete,
                executionComplete: !!execRes.execution_complete,
                businessComplete: !!execRes.business_complete,
                verifyOk: !!verifyRes.ok,
                verifyIssues: verifyRes.issues ?? [],
                completionScore: execRes.completion_score ?? null,
            });
            await loadRunDiagnostics(execRes.run_id);
            await loadDodHistory();
            const summaryLines = [
                `**Status**: ${execRes.status}`,
                `**Profile**: ${execRes.profile ?? resumeProfile}${execRes.collision_policy ? ` (collision=${execRes.collision_policy})` : ""}`,
                execRes.run_id ? `**Run ID**: ${execRes.run_id}` : "",
                execRes.resume_token ? `**Resume Token**: ${execRes.resume_token}` : "",
                `**Planner Complete**: ${execRes.planner_complete ? "yes" : "no"}`,
                `**Execution Complete**: ${execRes.execution_complete ? "yes" : "no"}`,
                `**Business Complete**: ${execRes.business_complete ? "yes" : "no"}`,
                `**Verify**: ${verifyRes.ok ? "ok" : "issues"}`,
                execRes.completion_score
                    ? `**Completion Score**: ${execRes.completion_score.score} (${execRes.completion_score.label})`
                    : "",
                execRes.resume_from != null ? `**Next Step**: ${execRes.resume_from + 1}` : "",
            ];
            const logLines = execRes.logs?.slice(0, 10).map(line => `- ${line}`) ?? [];
            const verifyLines = verifyRes.issues?.length ? verifyRes.issues.map(i => `- ${i}`) : [];
            const manualLines = execRes.manual_steps?.length
                ? execRes.manual_steps.map(step => `- ${step}`)
                : [];
            if (execRes.status === "approval_required" && execRes.approval?.action) {
                setPendingApproval({
                    planId: lastPlanId,
                    action: execRes.approval.action,
                    message: execRes.approval.message,
                    riskLevel: execRes.approval.risk_level,
                    policy: execRes.approval.policy,
                });
            } else {
                setPendingApproval(null);
            }
            setResults([{
                type: 'response',
                content: [
                    summaryLines.filter(Boolean).join("\n"),
                    logLines.length ? `\n**Logs**\n${logLines.join("\n")}` : "",
                    verifyLines.length ? `\n**Verify Issues**\n${verifyLines.join("\n")}` : "",
                    manualLines.length ? `\n**Manual Steps**\n${manualLines.join("\n")}` : ""
                ].join("\n")
            }]);
            triggerSuccess();
        } catch (error) {
            console.error("Resume failed", error);
            setRunPhase("failed");
            triggerError();
        } finally {
            setLoading(false);
        }
    };

    const handleResume = async () => {
        await resumeExecution(false);
    };

    const handleGuidedRecovery = async () => {
        if (firstFailedArtifactPath) {
            await openArtifactPath(firstFailedArtifactPath);
        }
        if (lastStatus === "manual_required" && lastPlanId && !pendingApproval) {
            await resumeExecution(true);
        }
    };

    const runRecoveryAction = async (action: RecoveryAction) => {
        if (recoveryActionBusyKey) return;
        setRecoveryActionBusyKey(action.key);
        let status: "completed" | "failed" = "completed";
        let details = action.description;
        try {
            if (action.kind === "preflight_fix" && action.fixAction) {
                await handlePreflightFix(action.fixAction);
                details = `preflight_fix:${action.fixAction}`;
            } else if (action.kind === "artifact" && action.path) {
                await openArtifactPath(action.path);
                details = `artifact:${action.path}`;
            } else if (action.kind === "guided_resume") {
                await handleGuidedRecovery();
                details = "guided_resume";
            }
        } catch (error) {
            status = "failed";
            const message = error instanceof Error ? error.message : String(error);
            details = `${details} (${message})`;
        } finally {
            if (action.kind !== "preflight_fix") {
                await recordRecoveryAction({
                    actionKey: action.assertionKey ?? `recovery.action.${action.key}`,
                    status,
                    details,
                    actual: status,
                });
            }
            setRecoveryActionBusyKey(null);
        }
    };

    const handleOneClickRecovery = async () => {
        if (recoveryActions.length === 0) return;
        await runRecoveryAction(recoveryActions[0]);
    };

    const handleApprovalDecision = async (decision: "allow_once" | "allow_always" | "deny") => {
        if (!pendingApproval) return;
        setApprovalBusy(true);
        setRunPhase("approval_required");
        try {
            const approvalRes = await agentApprove(
                pendingApproval.planId,
                pendingApproval.action,
                decision
            );
            if (decision === "deny" || approvalRes.status === "denied") {
                setResults([{
                    type: 'response',
                    content: `**Approval Denied**\n- Action: ${pendingApproval.action}\n- Policy: ${approvalRes.policy}`
                }]);
                setPendingApproval(null);
                setRunPhase("failed");
                triggerSuccess();
                return;
            }
            const approvalProfile = safeExecutionMode ? "strict" : activeExecutionProfile;
            const execRes = await agentExecute(pendingApproval.planId, approvalProfile, {
                resumeToken: runSnapshot?.resumeToken ?? null,
            });
            const verifyRes = await agentVerify(pendingApproval.planId);
            updateExecutionState({
                status: execRes.status,
                runId: execRes.run_id ?? null,
                resumeToken: execRes.resume_token ?? null,
                plannerComplete: !!execRes.planner_complete,
                executionComplete: !!execRes.execution_complete,
                businessComplete: !!execRes.business_complete,
                verifyOk: !!verifyRes.ok,
                verifyIssues: verifyRes.issues ?? [],
                completionScore: execRes.completion_score ?? null,
            });
            await loadRunDiagnostics(execRes.run_id);
            await loadDodHistory();
            const summaryLines = [
                `**Status**: ${execRes.status}`,
                `**Profile**: ${execRes.profile ?? approvalProfile}${execRes.collision_policy ? ` (collision=${execRes.collision_policy})` : ""}`,
                execRes.run_id ? `**Run ID**: ${execRes.run_id}` : "",
                execRes.resume_token ? `**Resume Token**: ${execRes.resume_token}` : "",
                `**Planner Complete**: ${execRes.planner_complete ? "yes" : "no"}`,
                `**Execution Complete**: ${execRes.execution_complete ? "yes" : "no"}`,
                `**Business Complete**: ${execRes.business_complete ? "yes" : "no"}`,
                `**Verify**: ${verifyRes.ok ? "ok" : "issues"}`,
                execRes.completion_score
                    ? `**Completion Score**: ${execRes.completion_score.score} (${execRes.completion_score.label})`
                    : "",
                execRes.resume_from != null ? `**Next Step**: ${execRes.resume_from + 1}` : "",
            ];
            const logLines = execRes.logs?.slice(0, 10).map(line => `- ${line}`) ?? [];
            const verifyLines = verifyRes.issues?.length ? verifyRes.issues.map(i => `- ${i}`) : [];
            const manualLines = execRes.manual_steps?.length
                ? execRes.manual_steps.map(step => `- ${step}`)
                : [];
            if (execRes.status === "approval_required" && execRes.approval?.action) {
                setPendingApproval({
                    planId: pendingApproval.planId,
                    action: execRes.approval.action,
                    message: execRes.approval.message,
                    riskLevel: execRes.approval.risk_level,
                    policy: execRes.approval.policy,
                });
            } else {
                setPendingApproval(null);
            }
            setResults([{
                type: 'response',
                content: [
                    summaryLines.filter(Boolean).join("\n"),
                    logLines.length ? `\n**Logs**\n${logLines.join("\n")}` : "",
                    verifyLines.length ? `\n**Verify Issues**\n${verifyLines.join("\n")}` : "",
                    manualLines.length ? `\n**Manual Steps**\n${manualLines.join("\n")}` : ""
                ].join("\n")
            }]);
            triggerSuccess();
        } catch (error) {
            console.error("Approval flow failed", error);
            setRunPhase("failed");
            triggerError();
        } finally {
            setApprovalBusy(false);
        }
    };

    const handleApprove = async (id: number) => {
        const now = Date.now();
        const last = approveCooldowns[id] ?? 0;
        if (now - last < 3000) {
            return;
        }
        setApproveCooldowns(prev => ({ ...prev, [id]: now }));
        setApproveErrors(prev => {
            const next = { ...prev };
            delete next[id];
            return next;
        });
        setApprovingIds(prev => new Set(prev).add(id));
        const sourceRec = (recs ?? []).find((rec) => rec.id === id) ?? null;
        addWatchRecommendation(id, sourceRec);
        setProvisioningUiState(id, "provisioning", {
            opId: provisioningUiByRecId[id]?.opId ?? null,
            detail: "requested",
        });
        try {
            const approved = await approveRecommendation(id);
            const workflowId = approved.workflow_id?.trim() || approved.id?.trim() || sourceRec?.workflow_id?.trim() || null;
            const workflowUrl = approved.workflow_url?.trim() || resolveRecommendationWorkflowUrl(sourceRec ?? null, workflowId);
            const provisionOpId = approved.provision_op_id ?? null;
            const provisionStatus = approved.provision_status?.trim() || null;

            if (workflowUrl) {
                clearProvisioningUiState(id);
                const busyKey = `approve:${id}`;
                setN8nOpenBusyKey(busyKey);
                try {
                    await openExternalTarget(workflowUrl);
                    setResults([
                        {
                            type: "response",
                            content: [
                                "**Workflow 승인 완료**",
                                `- recommendation_id: \`${id}\``,
                                workflowId ? `- workflow_id: \`${workflowId}\`` : "",
                                provisionOpId != null ? `- provision_op_id: \`${provisionOpId}\`` : "",
                                `- n8n 편집기: ${workflowUrl}`,
                                "- n8n 화면을 열어 생성 결과를 바로 확인하세요.",
                            ]
                                .filter(Boolean)
                                .join("\n"),
                        },
                    ]);
                } catch (openError) {
                    const openMsg =
                        openError instanceof Error ? openError.message : String(openError);
                    setResults([
                        {
                            type: "response",
                            content: [
                                "**Workflow 승인 완료 (열기 실패)**",
                                `- recommendation_id: \`${id}\``,
                                workflowId ? `- workflow_id: \`${workflowId}\`` : "",
                                provisionOpId != null ? `- provision_op_id: \`${provisionOpId}\`` : "",
                                `- n8n URL: ${workflowUrl}`,
                                `- 열기 오류: ${openMsg}`,
                            ]
                                .filter(Boolean)
                                .join("\n"),
                        },
                    ]);
                } finally {
                    setShowDetailPanel(true);
                    setN8nOpenBusyKey(null);
                }
                removeWatchRecommendation(id);
            } else {
                setProvisioningUiState(id, "provisioning", {
                    opId: provisionOpId,
                    detail: provisionStatus ?? "requested",
                });
                const busyKey = `approve:${id}`;
                setN8nOpenBusyKey(busyKey);
                let editorOpenNote = "- n8n 편집기 홈을 먼저 열었습니다. workflow URL 준비 시 상세 페이지를 자동으로 엽니다.";
                try {
                    await openExternalTarget(N8N_EDITOR_BASE_URL);
                } catch (openError) {
                    const openMsg =
                        openError instanceof Error ? openError.message : String(openError);
                    editorOpenNote = `- n8n 편집기 자동 열기 실패: ${openMsg}`;
                } finally {
                    setN8nOpenBusyKey(null);
                }
                setResults([
                    {
                        type: "response",
                        content: [
                            "**승인 완료 · 생성 대기**",
                            `- recommendation_id: \`${id}\``,
                            provisionOpId != null ? `- provision_op_id: \`${provisionOpId}\`` : "",
                            provisionStatus ? `- provision_status: \`${provisionStatus}\`` : "",
                            editorOpenNote,
                            "- workflow URL 생성 중입니다. 준비되면 해당 워크플로 편집기를 자동으로 엽니다.",
                        ].join("\n"),
                    },
                ]);
                setShowDetailPanel(true);
                void monitorApprovedWorkflow(id, sourceRec, provisionOpId);
            }
            triggerSuccess();
        } catch (e) {
            console.error("Approve failed", e);
            triggerError();
            const raw = extractErrorMessage(e);
            const mapped = mapApproveError(raw);
            setProvisioningUiState(id, "failed", { opId: null, detail: mapped });
            setApproveErrors(prev => ({ ...prev, [id]: mapped }));
            setResults([
                {
                    type: "response",
                    content: [
                        "**승인 요청 지연/실패 감지**",
                        `- recommendation_id: \`${id}\``,
                        `- 상세: ${mapped}`,
                        "- 백엔드 반영 여부를 확인하면서 workflow URL 생성을 추적합니다.",
                    ].join("\n"),
                },
            ]);
            setShowDetailPanel(true);
            void monitorApprovedWorkflow(id, sourceRec, null);
        } finally {
            setApprovingIds(prev => {
                const next = new Set(prev);
                next.delete(id);
                return next;
            });
            refetch();
        }
    };

    const mapApproveError = (raw: string) => {
        const msg = raw.toLowerCase();
        if (msg.includes("unauthorized") || msg.includes("401")) {
            return "n8n API 인증 실패 (API 키 확인 필요)";
        }
        if (msg.includes("nodes") && msg.includes("empty")) {
            return "워크플로우 노드가 비어 있음 (재시도 또는 최소 템플릿 확인)";
        }
        if (msg.includes("timeout")) {
            return "요청 시간이 초과됨. 승인 접수는 됐을 수 있으니 잠시 후 상태를 확인하세요.";
        }
        if (msg.includes("connection refused")) {
            return "코어 서버 연결 실패 (5680 실행 상태 확인)";
        }
        return raw;
    };

    // Keyboard Handler
    const handleKeyDown = async (e: React.KeyboardEvent<HTMLInputElement>) => {
        const nativeEvent = e.nativeEvent as KeyboardEvent;
        const composingMs = Date.now() - (composingSinceRef.current || Date.now());
        const composingHot = isComposing && composingMs < 1500;
        if (composingHot || nativeEvent.isComposing || nativeEvent.keyCode === 229) {
            return;
        }
        if (isComposing && !composingHot) {
            setIsComposing(false);
            composingSinceRef.current = 0;
        }
        if (isExecutionLocked && e.key === "Enter") {
            e.preventDefault();
            return;
        }
        if (e.key === "ArrowDown") {
            if (navigableItems.length === 0) return;
            e.preventDefault();
            setSelectedIndex(prev => (prev + 1) % navigableItems.length);
        } else if (e.key === "ArrowUp") {
            if (navigableItems.length === 0) return;
            e.preventDefault();
            setSelectedIndex(prev => (prev - 1 + navigableItems.length) % navigableItems.length);
        } else if (e.key === "Enter") {
            e.preventDefault();
            if (input.trim()) {
                await handleSend();
                return;
            }

            if (navigableItems.length > 0) {
                const selected = navigableItems[selectedIndex];
                if (selected && selected.type === 'recommendation') {
                    const rec = selected.data as { id: number; title: string; summary: string; status: string };
                    await handleApprove(rec.id);
                }
            }
        }
    };

    const handleInputPaste = (e: React.ClipboardEvent<HTMLInputElement>) => {
        const pasted = e.clipboardData.getData("text");
        if (!pasted) return;
        e.preventDefault();
        const normalized = pasted.replace(/\s+/g, " ").trim();
        const target = e.currentTarget;
        const start = target.selectionStart ?? target.value.length;
        const end = target.selectionEnd ?? target.value.length;
        const nextValue =
            target.value.slice(0, start) + normalized + target.value.slice(end);
        setInput(nextValue);
        requestAnimationFrame(() => {
            const caret = start + normalized.length;
            try {
                target.setSelectionRange(caret, caret);
            } catch {
                // no-op
            }
        });
    };

    const handleBackgroundClick = async (e: React.MouseEvent) => {
        if (e.target === e.currentTarget) {
            if (isExecutionLocked) return;
            try {
                const tauriMeta =
                    (window as WindowWithTauriMeta).__TAURI_METADATA__ ||
                    (window as WindowWithTauriMeta).__TAURI__?.metadata ||
                    (window as WindowWithTauriMeta).__TAURI_INTERNALS__?.metadata;
                if (tauriMeta) {
                    await getCurrentWindow().hide();
                }
            } catch (error) {
                console.error("Failed to hide window:", error);
            }
        }
    };

    const launcherWidthClass = compactLayoutMode
        ? "max-w-[calc(100vw-16px)] sm:max-w-[calc(100vw-24px)] lg:max-w-[1080px] xl:max-w-[1160px]"
        : "max-w-[calc(100vw-16px)] sm:max-w-[calc(100vw-24px)] lg:max-w-[1220px] xl:max-w-[1280px]";

    return (
        <div
            className="launcher-root w-full h-full bg-[radial-gradient(120%_90%_at_50%_0%,rgba(18,44,89,0.45),rgba(9,13,22,0.96)_62%,rgba(7,10,17,0.98)_100%)] flex items-end justify-center pb-2 sm:pb-3 px-2 sm:px-3 pointer-events-none"
            onMouseDown={handleBackgroundClick}
        >
            <motion.div
                className={`launcher-card pointer-events-auto w-full ${launcherWidthClass} max-h-[calc(100vh-6px)] bg-[#121722]/96 backdrop-blur-2xl rounded-[18px] shadow-2xl overflow-hidden border transition-colors duration-500
                    ${successPulse ? 'border-green-500/50 shadow-green-500/20' : 'border-white/10 ring-1 ring-black/5'}
                `}
                initial={{ scale: 0.9, opacity: 0 }}
                animate={{
                    scale: 1,
                    opacity: 1,
                    x: shake ? [0, -10, 10, -10, 10, 0] : 0
                }}
                transition={{ type: "spring", duration: 0.3 }}
            >
                {/* Composer */}
                <div className="launcher-composer px-4 py-3.5 bg-[#141820]">
                    <div className="launcher-top-row flex items-center gap-3">
                        <div className="launcher-mode-switch inline-flex items-center gap-1 rounded-xl bg-white/6 p-1 shrink-0">
                            <button
                                onClick={() => {
                                    setIsComposing(false);
                                    setComposerMode("nl");
                                }}
                                disabled={isExecutionLocked}
                                className={`px-3 py-1.5 text-xs rounded-lg font-medium transition-colors ${composerMode === "nl"
                                    ? "bg-white/15 text-white"
                                    : "text-gray-400 hover:text-gray-200"
                                    } ${isExecutionLocked ? "opacity-50 cursor-not-allowed" : ""}`}
                            >
                                자연어
                            </button>
                            <button
                                onClick={() => {
                                    setIsComposing(false);
                                    setComposerMode("chat");
                                    setShowDetailPanel(true);
                                }}
                                disabled={isExecutionLocked}
                                className={`px-3 py-1.5 text-xs rounded-lg font-medium transition-colors ${composerMode === "chat"
                                    ? "bg-white/15 text-white"
                                    : "text-gray-400 hover:text-gray-200"
                                    } ${isExecutionLocked ? "opacity-50 cursor-not-allowed" : ""}`}
                            >
                                대화
                            </button>
                            <button
                                onClick={() => {
                                    setIsComposing(false);
                                    setComposerMode("program");
                                }}
                                disabled={isExecutionLocked}
                                className={`px-3 py-1.5 text-xs rounded-lg font-medium transition-colors ${composerMode === "program"
                                    ? "bg-white/15 text-white"
                                    : "text-gray-400 hover:text-gray-200"
                                    } ${isExecutionLocked ? "opacity-50 cursor-not-allowed" : ""}`}
                            >
                                프로그램
                            </button>
                        </div>

                        {composerMode !== "chat" && (
                            <button
                                onClick={() => setShowAdvancedControls((prev) => !prev)}
                                disabled={isExecutionLocked}
                                className={`px-2.5 py-1.5 text-[11px] rounded-xl border transition-colors shrink-0 ${
                                    showAdvancedControls
                                        ? "border-cyan-400/35 bg-cyan-500/15 text-cyan-200"
                                        : "border-white/20 bg-white/5 text-gray-300 hover:bg-white/10"
                                } ${isExecutionLocked ? "opacity-50 cursor-not-allowed" : ""}`}
                                title="고급 실행 옵션/점검 패널 표시"
                            >
                                옵션 {showAdvancedControls ? "ON" : "OFF"}
                            </button>
                        )}

                        {composerMode !== "chat" && showAdvancedControls && (
                            <div className="inline-flex items-center gap-1 rounded-xl bg-white/6 p-1 shrink-0">
                                {EXECUTION_PROFILE_OPTIONS.map((option) => (
                                    <button
                                        key={option.value}
                                        onClick={() => setExecutionProfile(option.value)}
                                        disabled={
                                            isExecutionLocked ||
                                            (safeExecutionMode && option.value !== "strict")
                                        }
                                        title={`${option.label}: ${option.hint}`}
                                        className={`px-2.5 py-1.5 text-xs rounded-lg font-medium transition-colors ${
                                            executionProfile === option.value
                                                ? "bg-white/15 text-white"
                                                : "text-gray-400 hover:text-gray-200"
                                        } ${
                                            isExecutionLocked ||
                                            (safeExecutionMode && option.value !== "strict")
                                                ? "opacity-50 cursor-not-allowed"
                                                : ""
                                        }`}
                                    >
                                        {option.label}
                                    </button>
                                ))}
                            </div>
                        )}
                        {composerMode !== "chat" && showAdvancedControls && (
                            <div className="hidden lg:flex items-center gap-2 text-[11px] text-gray-300 shrink-0">
                                <span className="px-2 py-1 rounded-full border border-white/15 bg-white/5">
                                    권장 {profileLabel(profileRecommendation.profile)}
                                </span>
                                {executionProfile !== profileRecommendation.profile && (
                                    <button
                                        onClick={() => setExecutionProfile(profileRecommendation.profile)}
                                        disabled={isExecutionLocked}
                                        className="px-2 py-1 rounded-full border border-amber-400/35 bg-amber-500/15 text-amber-100 hover:bg-amber-500/25 disabled:opacity-50"
                                        title={profileRecommendation.reason}
                                    >
                                        권장 적용
                                    </button>
                                )}
                                <button
                                    onClick={() =>
                                        setAutoApplyRecommendedProfile((prev) => !prev)
                                    }
                                    disabled={isExecutionLocked || safeExecutionMode}
                                    className={`px-2 py-1 rounded-full border text-[11px] transition-colors ${
                                        autoApplyRecommendedProfile
                                            ? "border-emerald-400/35 bg-emerald-500/15 text-emerald-200"
                                            : "border-white/20 bg-white/5 text-gray-300"
                                    } ${
                                        isExecutionLocked || safeExecutionMode
                                            ? "opacity-50 cursor-not-allowed"
                                            : ""
                                    }`}
                                    title="Strict 차단 상황에서 권장 프로필로 자동 전환"
                                >
                                    자동 전환 {autoApplyRecommendedProfile ? "ON" : "OFF"}
                                </button>
                                <button
                                    onClick={() => setSafeExecutionMode((prev) => !prev)}
                                    disabled={isExecutionLocked}
                                    className={`px-2 py-1 rounded-full border text-[11px] transition-colors ${
                                        safeExecutionMode
                                            ? "border-sky-400/35 bg-sky-500/15 text-sky-200"
                                            : "border-white/20 bg-white/5 text-gray-300"
                                    } ${isExecutionLocked ? "opacity-50 cursor-not-allowed" : ""}`}
                                    title="안전 모드 ON이면 Strict 프로필로 고정하고 자동 전환을 잠급니다."
                                >
                                    안전 모드 {safeExecutionMode ? "ON" : "OFF"}
                                </button>
                                <button
                                    onClick={() => setCompactLayoutMode((prev) => !prev)}
                                    disabled={isExecutionLocked}
                                    className={`px-2 py-1 rounded-full border text-[11px] transition-colors ${
                                        compactLayoutMode
                                            ? "border-cyan-400/35 bg-cyan-500/15 text-cyan-200"
                                            : "border-white/20 bg-white/5 text-gray-300"
                                    } ${isExecutionLocked ? "opacity-50 cursor-not-allowed" : ""}`}
                                    title="컴팩트 레이아웃 ON이면 입력/결과 패널 높이를 줄여 집중도를 높입니다."
                                >
                                    컴팩트 {compactLayoutMode ? "ON" : "OFF"}
                                </button>
                            </div>
                        )}

                        <div className="launcher-input-wrap relative flex-1">
                            <input
                                ref={inputRef}
                                type="text"
                                className="launcher-input w-full h-11 sm:h-12 md:h-[50px] bg-white/[0.03] border border-white/15 rounded-xl px-3.5 sm:px-4 text-[16px] sm:text-[18px] md:text-[20px] text-white/95 placeholder-gray-500 outline-none focus:border-white/30 transition-colors"
                                placeholder={
                                    composerMode === "program"
                                        ? "버튼 또는 명령으로 실행"
                                        : composerMode === "chat"
                                          ? "간단히 대화해보세요"
                                          : "무엇이든 부탁하세요"
                                }
                                value={input}
                                onChange={(e) => setInput(e.target.value)}
                                onCompositionStart={() => {
                                    composingSinceRef.current = Date.now();
                                    setIsComposing(true);
                                }}
                                onCompositionEnd={() => {
                                    composingSinceRef.current = 0;
                                    setIsComposing(false);
                                }}
                                onBlur={() => setIsComposing(false)}
                                onPaste={handleInputPaste}
                                onKeyDown={handleKeyDown}
                                autoComplete="off"
                                autoCorrect="off"
                                autoCapitalize="none"
                                spellCheck={false}
                                autoFocus
                            />
                        </div>

                        <button
                            onClick={handleSend}
                            disabled={!input.trim() || isExecutionLocked || loading}
                            className="launcher-send w-11 h-11 sm:w-12 sm:h-12 rounded-full bg-white/18 hover:bg-white/30 disabled:opacity-40 text-white flex items-center justify-center transition-colors"
                        >
                            {loading ? (
                                <Activity className="w-5 h-5 animate-spin" />
                            ) : (
                                <ArrowUp className="w-5 h-5" />
                            )}
                        </button>
                        {pendingDispatch && (
                            <button
                                onClick={cancelPendingDispatch}
                                className="h-12 px-3 rounded-xl border border-rose-400/35 bg-rose-500/15 text-rose-100 hover:bg-rose-500/25 transition-colors text-xs"
                            >
                                취소 ({safeCountdownSeconds})
                            </button>
                        )}
                    </div>

                    {composerMode !== "chat" && executionLockHint && (
                        <div className="mt-2 rounded-lg border border-amber-400/30 bg-amber-500/10 px-3 py-2 text-[11px] text-amber-100">
                            실행 차단 사유: {executionLockHint}
                        </div>
                    )}

                    {composerMode !== "chat" && showPreflightPanel && (
                    <div className={`mt-2.5 rounded-xl border px-3 py-2 ${preflightOk === false || preflightError ? "border-rose-500/35 bg-rose-500/10" : "border-white/10 bg-[#1b1b1b]/80"}`}>
                        <div className="flex items-center justify-between gap-3">
                            <div className="text-xs text-gray-200">
                                실행 전 점검:{" "}
                                <span className={preflightOk ? "text-emerald-300" : preflightOk === false ? "text-rose-300" : "text-gray-400"}>
                                    {preflightLoading ? "점검 중..." : preflightOk ? "준비됨" : "차단됨"}
                                </span>
                                {preflightActiveApp && (
                                    <span className="text-gray-400 ml-2">front={preflightActiveApp}</span>
                                )}
                                {preflightCheckedAt && (
                                    <span className="text-gray-500 ml-2">{new Date(preflightCheckedAt).toLocaleTimeString()}</span>
                                )}
                                <div className="text-[11px] text-gray-400 mt-1">
                                    프로필 권장: {profileLabel(profileRecommendation.profile)} · {profileRecommendation.reason}
                                </div>
                            </div>
                            <div className="flex items-center gap-2">
                                <button
                                    onClick={() => void runPreflightCheck(false)}
                                    disabled={preflightLoading || isExecutionLocked}
                                    className="text-[11px] px-2.5 py-1 rounded-full border border-white/15 bg-white/5 hover:bg-white/10 disabled:opacity-50"
                                >
                                    다시 점검
                                </button>
                                <button
                                    onClick={() => setShowPreflightDetail((prev) => !prev)}
                                    className="text-[11px] px-2.5 py-1 rounded-full border border-white/15 bg-white/5 hover:bg-white/10"
                                >
                                    {showPreflightDetail ? "숨기기" : "상세"}
                                </button>
                            </div>
                        </div>
                        {showPreflightDetail && (
                            <div className="mt-2 grid grid-cols-1 md:grid-cols-3 gap-2">
                                {(focusPreflightBlocked ||
                                    (accessibilityPreflight && !accessibilityPreflight.ok) ||
                                    (screenCapturePreflight && !screenCapturePreflight.ok)) && (
                                        <div className="md:col-span-3 rounded border border-amber-400/30 bg-amber-500/10 px-2 py-2 text-[11px] text-amber-200">
                                            <div className="font-semibold mb-1">빠른 복구</div>
                                            <div className="flex flex-wrap gap-1.5">
                                                {focusPreflightBlocked && (
                                                    <button
                                                        onClick={() => void handlePreflightFix("prepare_isolated_mode")}
                                                        disabled={!!preflightFixBusy || preflightLoading || isExecutionLocked}
                                                        className="px-2 py-1 rounded border border-amber-300/40 bg-amber-400/20 hover:bg-amber-400/30 disabled:opacity-50"
                                                    >
                                                        {preflightFixBusy === "prepare_isolated_mode" ? "처리 중..." : "격리 모드 준비"}
                                                    </button>
                                                )}
                                                {focusPreflightBlocked && (
                                                    <button
                                                        onClick={() => void handlePreflightFix("activate_finder")}
                                                        disabled={!!preflightFixBusy || preflightLoading || isExecutionLocked}
                                                        className="px-2 py-1 rounded border border-amber-300/40 bg-amber-400/20 hover:bg-amber-400/30 disabled:opacity-50"
                                                    >
                                                        {preflightFixBusy === "activate_finder" ? "처리 중..." : "Finder 전면 복구"}
                                                    </button>
                                                )}
                                                {accessibilityPreflight && !accessibilityPreflight.ok && (
                                                    <button
                                                        onClick={() => void handlePreflightFix("open_accessibility_settings")}
                                                        disabled={!!preflightFixBusy || preflightLoading || isExecutionLocked}
                                                        className="px-2 py-1 rounded border border-amber-300/40 bg-amber-400/20 hover:bg-amber-400/30 disabled:opacity-50"
                                                    >
                                                        {preflightFixBusy === "open_accessibility_settings" ? "처리 중..." : "접근성 설정 열기"}
                                                    </button>
                                                )}
                                                {screenCapturePreflight && !screenCapturePreflight.ok && (
                                                    <button
                                                        onClick={() => void handlePreflightFix("open_screen_capture_settings")}
                                                        disabled={!!preflightFixBusy || preflightLoading || isExecutionLocked}
                                                        className="px-2 py-1 rounded border border-amber-300/40 bg-amber-400/20 hover:bg-amber-400/30 disabled:opacity-50"
                                                    >
                                                        {preflightFixBusy === "open_screen_capture_settings" ? "처리 중..." : "화면 기록 설정 열기"}
                                                    </button>
                                                )}
                                                {screenCapturePreflight && !screenCapturePreflight.ok && (
                                                    <button
                                                        onClick={() => void handlePreflightFix("reveal_core_binary")}
                                                        disabled={!!preflightFixBusy || preflightLoading || isExecutionLocked}
                                                        className="px-2 py-1 rounded border border-amber-300/40 bg-amber-400/20 hover:bg-amber-400/30 disabled:opacity-50"
                                                    >
                                                        {preflightFixBusy === "reveal_core_binary" ? "처리 중..." : "코어 파일 보기"}
                                                    </button>
                                                )}
                                                {screenCapturePreflight && !screenCapturePreflight.ok && (
                                                    <button
                                                        onClick={() => void handlePreflightFix("request_screen_capture_access")}
                                                        disabled={!!preflightFixBusy || preflightLoading || isExecutionLocked}
                                                        className="px-2 py-1 rounded border border-amber-300/40 bg-amber-400/20 hover:bg-amber-400/30 disabled:opacity-50"
                                                    >
                                                        {preflightFixBusy === "request_screen_capture_access" ? "처리 중..." : "권한 요청"}
                                                    </button>
                                                )}
                                            </div>
                                            {preflightFixMessage && (
                                                <div className="mt-1 text-amber-100/90">{preflightFixMessage}</div>
                                            )}
                                        </div>
                                    )}
                                {preflightChecks.map((check) => (
                                    <div
                                        key={check.key}
                                        className={`text-[11px] rounded border px-2 py-1.5 ${check.ok ? "border-emerald-400/30 bg-emerald-500/10 text-emerald-200" : "border-rose-400/30 bg-rose-500/10 text-rose-200"}`}
                                    >
                                        <div className="font-semibold">{check.ok ? "✅" : "❌"} {check.label}</div>
                                        <div className="opacity-80 mt-0.5">{check.message}</div>
                                        {(check.expected || check.actual) && (
                                            <div className="opacity-70 mt-0.5">
                                                expected={check.expected ?? "-"} / actual={check.actual ?? "-"}
                                            </div>
                                        )}
                                    </div>
                                ))}
                                {preflightError && (
                                    <div className="text-[11px] rounded border border-rose-400/30 bg-rose-500/10 text-rose-200 px-2 py-1.5 md:col-span-3">
                                        API error: {preflightError}
                                    </div>
                                )}
                                {focusPreflightBlocked && (
                                    <div className="text-[11px] rounded border border-amber-400/30 bg-amber-500/10 text-amber-200 px-2 py-1.5 md:col-span-3">
                                        권장: 실행 중 전면 앱 충돌을 피하려면 전용 데스크톱/사용자 세션에서 실행하세요.
                                    </div>
                                )}
                            </div>
                        )}
                    </div>
                    )}

                    {composerMode === "nl" && !input.trim() && (
                        <div className="launcher-quick-strip mt-2.5 rounded-xl border border-white/10 bg-[#1b1b1b]/90 p-2.5 flex flex-nowrap gap-2 overflow-x-auto">
                            {(compactLayoutMode ? QUICK_NL_SUGGESTIONS.slice(0, 2) : QUICK_NL_SUGGESTIONS).map((suggestion) => (
                                <button
                                    key={suggestion}
                                    onClick={() => handleSuggestionClick(suggestion)}
                                    disabled={isExecutionLocked}
                                    className="text-xs px-3.5 py-1.5 rounded-full bg-white/8 text-gray-200 hover:bg-white/15 border border-white/10 transition-colors whitespace-nowrap disabled:opacity-50 disabled:cursor-not-allowed"
                                >
                                    {suggestion}
                                </button>
                            ))}
                        </div>
                    )}

                    {composerMode === "chat" && !input.trim() && (
                        <div className="launcher-quick-strip mt-2.5 rounded-xl border border-white/10 bg-[#1b1b1b]/90 p-2.5 flex flex-nowrap gap-2 overflow-x-auto">
                            {(compactLayoutMode ? QUICK_CHAT_SUGGESTIONS.slice(0, 2) : QUICK_CHAT_SUGGESTIONS).map((suggestion) => (
                                <button
                                    key={suggestion}
                                    onClick={() => handleSuggestionClick(suggestion)}
                                    disabled={isExecutionLocked}
                                    className="text-xs px-3.5 py-1.5 rounded-full bg-white/8 text-gray-200 hover:bg-white/15 border border-white/10 transition-colors whitespace-nowrap disabled:opacity-50 disabled:cursor-not-allowed"
                                >
                                    {suggestion}
                                </button>
                            ))}
                        </div>
                    )}

                    {composerMode === "program" && (
                        <div className="launcher-quick-strip mt-2.5 rounded-xl border border-white/10 bg-[#1b1b1b]/90 p-2.5 flex flex-nowrap gap-2 overflow-x-auto">
                            {(compactLayoutMode ? QUICK_PROGRAM_ACTIONS.slice(0, 4) : QUICK_PROGRAM_ACTIONS).map((action) => (
                                <button
                                    key={action.key}
                                    onClick={() => handleQuickProgramAction(action)}
                                    disabled={isExecutionLocked}
                                    className="text-xs px-3.5 py-1.5 rounded-full bg-white/8 text-gray-100 hover:bg-white/15 border border-white/10 disabled:opacity-50 whitespace-nowrap"
                                >
                                    {action.label}
                                </button>
                            ))}
                        </div>
                    )}

                    <div className="launcher-toolbar mt-2.5 flex items-center justify-between text-gray-300">
                        <div className="flex items-center gap-1.5">
                            <button
                                onClick={() =>
                                    setComposerMode((prev) =>
                                        prev === "nl" ? "chat" : prev === "chat" ? "program" : "nl"
                                    )
                                }
                                disabled={isExecutionLocked}
                                className="p-2 rounded-full hover:bg-white/10 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                                title="모드 순환"
                            >
                                <Plus className="w-4 h-4" />
                            </button>
                            <button
                                onClick={() => {
                                    setComposerMode("nl");
                                    setInput(prev => prev.trim() ? `웹 검색: ${prev}` : "웹 검색: ");
                                    inputRef.current?.focus();
                                }}
                                disabled={isExecutionLocked}
                                className="p-2 rounded-full hover:bg-white/10 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                                title="웹 검색 템플릿"
                            >
                                <Globe className="w-4 h-4" />
                            </button>
                            <button
                                onClick={() => {
                                    setComposerMode("nl");
                                    setInput(prev => prev.trim() ? `요약해줘: ${prev}` : "요약해줘: ");
                                    inputRef.current?.focus();
                                }}
                                disabled={isExecutionLocked}
                                className="p-2 rounded-full hover:bg-white/10 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                                title="요약 템플릿"
                            >
                                <Wand2 className="w-4 h-4" />
                            </button>
                            <button
                                onClick={() => setComposerMode("program")}
                                disabled={isExecutionLocked}
                                className="p-2 rounded-full hover:bg-white/10 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                                title="프로그램 버튼"
                            >
                                <AppWindow className="w-4 h-4" />
                            </button>
                            <button
                                onClick={() => setComposerMode("chat")}
                                disabled={isExecutionLocked}
                                className="p-2 rounded-full hover:bg-white/10 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                                title="대화 모드"
                            >
                                <MessageCircle className="w-4 h-4" />
                            </button>
                            <button
                                onClick={() => void handleTelegramListenerCommand("telegram listener start")}
                                disabled={loading || isExecutionLocked}
                                className="text-[11px] px-2.5 py-1 rounded-full border border-sky-400/30 bg-sky-500/15 text-sky-200 hover:bg-sky-500/25 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                                title="텔레그램 리스너 시작"
                            >
                                TG 시작
                            </button>
                            <button
                                onClick={() => void handleTelegramListenerCommand("telegram listener status")}
                                disabled={loading || isExecutionLocked}
                                className="text-[11px] px-2.5 py-1 rounded-full border border-white/20 bg-white/5 text-gray-200 hover:bg-white/10 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                                title="텔레그램 리스너 상태"
                            >
                                TG 상태
                            </button>
                            <span className="text-sm font-semibold tracking-wide text-gray-200 ml-1">
                                <span className="text-cyan-300 font-extrabold">A</span>llv
                                <span className="text-cyan-300 font-extrabold">I</span>a
                            </span>
                            <span className="text-xl font-semibold text-gray-300">5.2</span>
                            {runtimeInfo && (
                                <span
                                    className={`text-[11px] px-2 py-0.5 rounded-full border ${
                                        coreBinaryKind === "workspace"
                                            ? "border-emerald-400/35 bg-emerald-500/15 text-emerald-200"
                                            : coreBinaryKind === "bundle"
                                              ? "border-amber-400/35 bg-amber-500/15 text-amber-100"
                                              : "border-white/20 bg-white/5 text-gray-300"
                                    }`}
                                    title={`pid=${runtimeInfo.pid} | ${runtimeInfo.binary_path ?? "unknown"}`}
                                >
                                    core {runtimeInfo.version} · {coreBinaryKind}
                                </span>
                            )}
                        </div>

                        <div className="flex items-center gap-1.5">
                            {!isChatComposerMode && hasDetailContent && (
                                <button
                                    onClick={() => setShowDetailPanel(prev => !prev)}
                                    className="text-xs px-2.5 py-1.5 rounded-full border border-white/15 bg-white/5 hover:bg-white/10 transition-colors"
                                >
                                    {showDetailPanel ? "결과 숨기기" : `결과 보기${suggestionRecs.length > 0 ? ` (${suggestionRecs.length})` : ""}`}
                                </button>
                            )}
                            {runScore && (
                                <span className={`text-xs px-2.5 py-1.5 rounded-full border ${runScore.pass
                                    ? "border-emerald-400/40 bg-emerald-500/15 text-emerald-200"
                                    : "border-rose-400/40 bg-rose-500/15 text-rose-200"
                                    }`}>
                                    완성도 {runScore.score} · {runScore.label}
                                </span>
                            )}
                            <button
                                className="p-2 rounded-full hover:bg-white/10 text-gray-300 transition-colors"
                                title="record"
                            >
                                <Circle className="w-6 h-6" />
                            </button>
                            <button
                                className="p-2 rounded-full hover:bg-white/10 text-gray-300 transition-colors"
                                title="voice"
                            >
                                <Mic className="w-6 h-6" />
                            </button>
                        </div>
                    </div>
                </div>

                <div className="launcher-statusbar px-4 py-2 border-t border-white/10 bg-[#10141b] flex items-center justify-between">
                    <div className="flex items-center gap-2">
                        <span className={`h-2 w-2 rounded-full ${currentHud.dot}`} />
                        <span className={`text-[11px] px-2 py-0.5 rounded-full border ${currentHud.chip}`}>
                            {currentHud.label}
                        </span>
                        {runSnapshot?.runId && (
                            <span className="inline-flex items-center gap-1.5 text-[11px] text-gray-500">
                                <span>run: {runSnapshot.runId}</span>
                                <button
                                    onClick={() => void copyTextValue(runSnapshot.runId ?? "", "run_id")}
                                    className="px-1.5 py-0.5 rounded border border-white/15 text-gray-300 hover:bg-white/10"
                                >
                                    복사
                                </button>
                            </span>
                        )}
                        <span
                            className={`text-[11px] px-2 py-0.5 rounded-full border ${
                                safeExecutionMode
                                    ? "border-sky-400/35 bg-sky-500/15 text-sky-200"
                                    : "border-white/20 bg-white/5 text-gray-400"
                            }`}
                        >
                            안전 {safeExecutionMode ? "ON" : "OFF"}
                        </span>
                        {runtimeInfoError && (
                            <span className="text-[11px] px-2 py-0.5 rounded-full border border-rose-400/35 bg-rose-500/15 text-rose-200">
                                runtime info unavailable
                            </span>
                        )}
                        {isDevBundleMismatch && (
                            <span className="text-[11px] px-2 py-0.5 rounded-full border border-amber-400/35 bg-amber-500/15 text-amber-100">
                                DEV 코드와 실행 코어가 다를 수 있음
                            </span>
                        )}
                    </div>
                    {runSnapshot && (
                        <div className="text-[11px] text-gray-500 text-right">
                            <div>status={runSnapshot.status}</div>
                            {(runPhase === "running" || runPhase === "retrying") && (
                                <div className="text-amber-300/90">입력 충돌 감지 시 자동 pause/abort</div>
                            )}
                        </div>
                    )}
                </div>

                <AnimatePresence>
                    {shouldRenderDetailPanel && (
                        <motion.div
                            initial={{ opacity: 0, height: 0 }}
                            animate={{ opacity: 1, height: "auto" }}
                            exit={{ opacity: 0, height: 0 }}
                            className="launcher-detail-panel border-t border-white/10 bg-[#121212]/96"
                        >
                            <div className="px-4 py-3 border-b border-white/5 bg-[#171717]">
                                <div className="text-[11px] uppercase tracking-wider text-indigo-300 font-semibold mb-2">
                                    핵심 요약
                                </div>
                                <div className="grid grid-cols-1 md:grid-cols-3 gap-2">
                                    <div className="text-xs rounded border border-white/15 bg-white/5 px-2 py-1.5 text-gray-200">
                                        <div className="font-semibold">최종 상태</div>
                                        <div className="mt-0.5">
                                            {runPhase} {runSnapshot?.status ? `(${runSnapshot.status})` : ""}
                                        </div>
                                    </div>
                                    <div className="text-xs rounded border border-white/15 bg-white/5 px-2 py-1.5 text-gray-200">
                                        <div className="font-semibold">완성도</div>
                                        <div className="mt-0.5">
                                            {runScore ? `${runScore.score} · ${runScore.label}` : "n/a"}
                                        </div>
                                    </div>
                                    <div className="text-xs rounded border border-white/15 bg-white/5 px-2 py-1.5 text-gray-200">
                                        <div className="font-semibold">다음 액션</div>
                                        <div className="mt-0.5 line-clamp-2">{nextActionHint}</div>
                                    </div>
                                </div>
                                {recoveryActions.length > 0 && (
                                    <div className="mt-2 rounded border border-white/10 bg-white/5 px-2 py-2">
                                        <div className="text-[11px] uppercase tracking-wider text-amber-300 font-semibold mb-1">
                                            복구 액션
                                        </div>
                                        <div className="mb-2">
                                            <button
                                                onClick={() => void handleOneClickRecovery()}
                                                disabled={loading || !!recoveryActionBusyKey}
                                                className="text-[11px] px-2.5 py-1 rounded-full border border-sky-400/40 bg-sky-500/15 text-sky-100 hover:bg-sky-500/25 disabled:opacity-50"
                                                title={recoveryActions[0]?.description ?? "권장 우선 복구 실행"}
                                            >
                                                {recoveryActionBusyKey
                                                    ? "즉시 복구 실행..."
                                                    : `즉시 복구 실행: ${recoveryActions[0]?.label ?? "권장 액션"}`}
                                            </button>
                                        </div>
                                        <div className="flex flex-wrap gap-1.5">
                                            {recoveryActions.map((action) => (
                                                <button
                                                    key={action.key}
                                                    onClick={() => void runRecoveryAction(action)}
                                                    disabled={
                                                        loading ||
                                                        !!recoveryActionBusyKey ||
                                                        preflightFixBusy === action.fixAction ||
                                                        artifactOpenBusy === action.path
                                                    }
                                                    className="text-[11px] px-2.5 py-1 rounded-full border border-amber-400/35 bg-amber-500/15 text-amber-100 hover:bg-amber-500/25 disabled:opacity-50"
                                                    title={action.description}
                                                >
                                                    {recoveryActionBusyKey === action.key
                                                        ? `${action.label}...`
                                                        : action.label}
                                                </button>
                                            ))}
                                        </div>
                                    </div>
                                )}
                                <div className="mt-2 flex items-center justify-end gap-2">
                                    <button
                                        onClick={() => void loadDodHistory()}
                                        disabled={dodHistoryLoading}
                                        className="text-[11px] px-2.5 py-1 rounded-full border border-sky-400/35 bg-sky-500/15 text-sky-100 hover:bg-sky-500/25 disabled:opacity-50"
                                    >
                                        {dodHistoryLoading ? "히스토리 새로고침..." : "히스토리 새로고침"}
                                    </button>
                                    <button
                                        onClick={() => setShowDiagnostics((prev) => !prev)}
                                        className="text-[11px] px-2.5 py-1 rounded-full border border-white/15 bg-white/5 hover:bg-white/10"
                                    >
                                        {showDiagnostics ? "진단 접기" : "진단 펼치기"}
                                    </button>
                                </div>
                            </div>

                            {showDiagnostics && (
                                <div className="px-4 py-3 border-b border-white/5 bg-[#151515]">
                                    <div className="mb-2 rounded border border-white/10 bg-white/5 px-2 py-2">
                                        <div className="text-[11px] uppercase tracking-wider text-cyan-200 font-semibold mb-1">
                                            Singleton Lock Telemetry
                                        </div>
                                        {lockMetrics ? (
                                            <div className="grid grid-cols-2 md:grid-cols-5 gap-1.5 text-[11px]">
                                                <div className="rounded border border-emerald-400/25 bg-emerald-500/10 px-2 py-1 text-emerald-200">
                                                    acquired={lockMetrics.acquired}
                                                </div>
                                                <div className="rounded border border-sky-400/25 bg-sky-500/10 px-2 py-1 text-sky-200">
                                                    bypassed={lockMetrics.bypassed}
                                                </div>
                                                <div className="rounded border border-rose-400/25 bg-rose-500/10 px-2 py-1 text-rose-200">
                                                    blocked={lockMetrics.blocked}
                                                </div>
                                                <div className="rounded border border-amber-400/25 bg-amber-500/10 px-2 py-1 text-amber-100">
                                                    stale_recovered={lockMetrics.stale_recovered}
                                                </div>
                                                <div className="rounded border border-fuchsia-400/25 bg-fuchsia-500/10 px-2 py-1 text-fuchsia-200">
                                                    rejected={lockMetrics.rejected}
                                                </div>
                                            </div>
                                        ) : (
                                            <div className="text-xs text-gray-400">
                                                {lockMetricsError ?? "아직 lock telemetry를 불러오지 않았습니다."}
                                            </div>
                                        )}
                                    </div>
                                    <div className="mb-2 rounded border border-white/10 bg-white/5 px-2 py-2">
                                        <div className="text-[11px] uppercase tracking-wider text-cyan-200 font-semibold mb-1">
                                            Runtime Core Info
                                        </div>
                                        {runtimeInfo ? (
                                            <div className="grid grid-cols-1 md:grid-cols-2 gap-1.5 text-[11px]">
                                                <div className="rounded border border-white/15 bg-black/20 px-2 py-1 text-gray-200">
                                                    service={runtimeInfo.service} · version={runtimeInfo.version} · profile={runtimeInfo.profile}
                                                </div>
                                                <div className="rounded border border-white/15 bg-black/20 px-2 py-1 text-gray-200">
                                                    pid={runtimeInfo.pid} · port={runtimeInfo.api_port} · no_key={runtimeInfo.allow_no_key ? "1" : "0"}
                                                </div>
                                                <div className="rounded border border-white/15 bg-black/20 px-2 py-1 text-gray-300 md:col-span-2 break-all">
                                                    binary={runtimeInfo.binary_path ?? "unknown"}
                                                </div>
                                                <div className="rounded border border-white/15 bg-black/20 px-2 py-1 text-gray-400 md:col-span-2 break-all">
                                                    cwd={runtimeInfo.current_dir ?? "unknown"} · started_at={runtimeInfo.started_at}
                                                </div>
                                            </div>
                                        ) : (
                                            <div className="text-xs text-gray-400">
                                                {runtimeInfoError ?? "아직 runtime info를 불러오지 않았습니다."}
                                            </div>
                                        )}
                                    </div>
                                    <div className="text-[11px] uppercase tracking-wider text-cyan-300 font-semibold mb-2">
                                        최근 DoD 히스토리
                                    </div>
                                    {dodHistory.length === 0 ? (
                                        <div className="text-xs text-gray-400">
                                            {dodHistoryLoading
                                                ? "히스토리를 불러오는 중..."
                                                : "기록된 실행 히스토리가 없습니다."}
                                        </div>
                                    ) : (
                                        <div className="grid grid-cols-1 md:grid-cols-2 gap-2">
                                            {dodHistory.map((item) => {
                                                const assertionsPass =
                                                    item.assertionTotal === 0 || item.assertionFailed === 0;
                                                const stagePass =
                                                    item.plannerComplete &&
                                                    item.executionComplete &&
                                                    item.businessComplete;
                                                const pass = stagePass && assertionsPass;
                                                return (
                                                    <div
                                                        key={item.runId}
                                                        className={`text-xs rounded border px-2 py-1.5 ${
                                                            pass
                                                                ? "border-emerald-400/30 bg-emerald-500/10 text-emerald-200"
                                                                : "border-rose-400/30 bg-rose-500/10 text-rose-200"
                                                        }`}
                                                    >
                                                        <div className="font-semibold">
                                                            {pass ? "✅" : "❌"} {item.runId}
                                                        </div>
                                                        <div className="opacity-80 mt-0.5">
                                                            status={item.status} · assertions=
                                                            {item.assertionTotal - item.assertionFailed}/
                                                            {item.assertionTotal}
                                                        </div>
                                                        <div className="opacity-70 mt-0.5">
                                                            planner={item.plannerComplete ? "1" : "0"} · execution=
                                                            {item.executionComplete ? "1" : "0"} · business=
                                                            {item.businessComplete ? "1" : "0"}
                                                        </div>
                                                    </div>
                                                );
                                            })}
                                        </div>
                                    )}
                                    <div className="mt-2 rounded border border-white/10 bg-white/5 px-2 py-2">
                                        <div className="text-[11px] uppercase tracking-wider text-rose-200 font-semibold mb-1">
                                            반복 실패 Top 3
                                        </div>
                                        {dodFailureTop.length === 0 ? (
                                            <div className="text-xs text-gray-400">최근 반복 실패 항목이 없습니다.</div>
                                        ) : (
                                            <div className="space-y-1">
                                                {dodFailureTop.map((item, idx) => (
                                                    <div
                                                        key={`top-fail-${item.key}`}
                                                        className="text-xs rounded border border-rose-400/25 bg-rose-500/10 px-2 py-1 text-rose-100"
                                                    >
                                                        <div className="flex items-center justify-between gap-2">
                                                            <div className="font-semibold">
                                                                {idx + 1}. {item.key} ({item.count})
                                                            </div>
                                                            {recoveryActionForFailureKey(item.key) && (
                                                                <button
                                                                    onClick={() => {
                                                                        const action =
                                                                            recoveryActionForFailureKey(item.key);
                                                                        if (action) {
                                                                            void runRecoveryAction(action);
                                                                        }
                                                                    }}
                                                                    disabled={loading || !!recoveryActionBusyKey}
                                                                    className="text-[10px] px-2 py-0.5 rounded-full border border-rose-300/40 bg-rose-400/20 hover:bg-rose-400/30 disabled:opacity-50"
                                                                >
                                                                    복구 실행
                                                                </button>
                                                            )}
                                                        </div>
                                                        <div className="opacity-80 mt-0.5">
                                                            actual={item.sampleActual || "n/a"}
                                                        </div>
                                                    </div>
                                                ))}
                                            </div>
                                        )}
                                    </div>
                                </div>
                            )}

                            {showDiagnostics && dodItems.length > 0 && (
                                <div className="px-4 py-3 border-b border-white/5 bg-[#181818]">
                                    <div className="text-[11px] uppercase tracking-wider text-emerald-300 font-semibold mb-2">
                                        Stage DoD
                                    </div>
                                    <div className="grid grid-cols-1 md:grid-cols-2 gap-2">
                                        {dodItems.map((item) => (
                                            <div
                                                key={item.key}
                                                className={`text-xs rounded border px-2 py-1.5 ${item.done
                                                    ? "border-emerald-400/30 bg-emerald-500/10 text-emerald-200"
                                                    : "border-rose-400/30 bg-rose-500/10 text-rose-200"
                                                    }`}
                                            >
                                                <div className="font-semibold">{item.done ? "✅" : "❌"} {item.label}</div>
                                                <div className="opacity-80 mt-0.5">{item.detail}</div>
                                            </div>
                                        ))}
                                    </div>
                                </div>
                            )}

                            {showDiagnostics &&
                                (stageRuns.length > 0 ||
                                    stageAssertions.length > 0 ||
                                    taskRunArtifacts.length > 0) && (
                                <div className="px-4 py-3 border-b border-white/5 bg-[#151515]">
                                    <div className="text-[11px] uppercase tracking-wider text-sky-300 font-semibold mb-2">
                                        Persisted Stage Trace
                                    </div>
                                    {artifactActionMessage && (
                                        <div className="text-[11px] rounded border border-white/15 bg-white/5 px-2 py-1.5 text-gray-200 mb-2">
                                            {artifactActionMessage}
                                        </div>
                                    )}
                                    {recoveryAssertions.length > 0 && (
                                        <div className="mb-2 rounded border border-amber-400/25 bg-amber-500/10 px-2 py-2">
                                            <div className="text-[11px] uppercase tracking-wider text-amber-200 font-semibold mb-1">
                                                Recovery Timeline
                                            </div>
                                            <div className="space-y-1">
                                                {recoveryAssertions.map((item) => (
                                                    <div
                                                        key={`recovery-${item.id}`}
                                                        className={`text-[11px] rounded px-2 py-1 border ${
                                                            item.passed
                                                                ? "border-emerald-400/30 bg-emerald-500/10 text-emerald-200"
                                                                : "border-rose-400/30 bg-rose-500/10 text-rose-200"
                                                        }`}
                                                    >
                                                        <div className="font-medium">
                                                            {item.passed ? "✅" : "❌"} {item.assertion_key}
                                                        </div>
                                                        <div className="opacity-80">
                                                            expected={item.expected} actual={item.actual}
                                                        </div>
                                                        {item.evidence && (
                                                            <div className="opacity-75 line-clamp-1">{item.evidence}</div>
                                                        )}
                                                        <div className="opacity-60">
                                                            {new Date(item.created_at).toLocaleTimeString()}
                                                        </div>
                                                    </div>
                                                ))}
                                            </div>
                                        </div>
                                    )}
                                    {(runPhase === "failed" || lastStatus === "manual_required") && firstFailedArtifactPath && (
                                        <div className="mb-2">
                                            <button
                                                onClick={() => void handleGuidedRecovery()}
                                                disabled={loading || artifactOpenBusy === firstFailedArtifactPath}
                                                className="text-[11px] px-2.5 py-1.5 rounded border border-sky-400/40 bg-sky-500/15 text-sky-100 hover:bg-sky-500/25 disabled:opacity-50"
                                                title={firstFailedArtifactPath}
                                            >
                                                {lastStatus === "manual_required"
                                                    ? "실패 증거 열고 Resume"
                                                    : `실패 증거 열기 (${artifactPathLabel(firstFailedArtifactPath)})`}
                                            </button>
                                        </div>
                                    )}
                                    {stageTraceItems.length > 0 && (
                                        <div className="grid grid-cols-1 md:grid-cols-2 gap-2 mb-2">
                                            {stageTraceItems.map((row) => (
                                                <div
                                                    key={`stage-${row.stage.id}`}
                                                    className={`text-xs rounded border px-2 py-1.5 ${
                                                        row.stage.status === "completed"
                                                            ? "border-emerald-400/30 bg-emerald-500/10 text-emerald-200"
                                                            : row.stage.status === "running"
                                                              ? "border-sky-400/30 bg-sky-500/10 text-sky-200"
                                                              : row.stage.status === "retrying"
                                                                ? "border-amber-400/30 bg-amber-500/10 text-amber-200"
                                                                : row.stage.status === "blocked"
                                                                  ? "border-amber-500/40 bg-amber-600/10 text-amber-100"
                                                                  : "border-rose-400/30 bg-rose-500/10 text-rose-200"
                                                    }`}
                                                >
                                                    <div className="font-semibold">
                                                        {row.stage.status === "completed"
                                                            ? "✅"
                                                            : row.stage.status === "running"
                                                              ? "⏳"
                                                              : row.stage.status === "retrying"
                                                                ? "🔁"
                                                                : row.stage.status === "blocked"
                                                                  ? "⛔"
                                                                  : "❌"}{" "}
                                                        {row.stage.stage_order}. {row.stage.stage_name}
                                                    </div>
                                                    <div className="opacity-80 mt-0.5">
                                                        {row.stage.status} · assertions {row.assertions.length} · failed {row.failed.length}
                                                    </div>
                                                    {(row.stage.retry_count ?? 0) > 0 && (
                                                        <div className="opacity-80 mt-0.5">
                                                            retry {row.stage.retry_count}
                                                            {typeof row.stage.max_retries === "number" && row.stage.max_retries > 0
                                                                ? `/${row.stage.max_retries}`
                                                                : ""}
                                                            {row.stage.next_retry_at
                                                                ? ` · next=${new Date(row.stage.next_retry_at).toLocaleTimeString()}`
                                                                : ""}
                                                        </div>
                                                    )}
                                                    {row.stage.details && (
                                                        <div className="opacity-70 mt-0.5 line-clamp-2">{row.stage.details}</div>
                                                    )}
                                                    {row.failed.length > 0 && (
                                                        <div className="mt-1.5 space-y-1">
                                                            {row.failed.slice(0, 2).map((a) => {
                                                                const artifactPaths = extractArtifactPaths(
                                                                    a.evidence,
                                                                    row.stage.details,
                                                                    `${a.actual} ${a.expected}`
                                                                );
                                                                return (
                                                                    <div key={`stage-${row.stage.id}-assert-${a.id}`} className="opacity-90">
                                                                        <div>
                                                                            • {a.assertion_key}: expected={a.expected} actual={a.actual}
                                                                            {a.evidence ? ` | evidence=${compactEvidence(a.evidence)}` : ""}
                                                                        </div>
                                                                        {artifactPaths.length > 0 && (
                                                                            <div className="mt-1 flex flex-wrap gap-1">
                                                                                {artifactPaths.map((artifactPath) => (
                                                                                    <button
                                                                                        key={`open-${a.id}-${artifactPath}`}
                                                                                        onClick={() => void openArtifactPath(artifactPath)}
                                                                                        disabled={artifactOpenBusy === artifactPath}
                                                                                        className="text-[10px] px-2 py-0.5 rounded border border-sky-400/40 bg-sky-500/15 text-sky-100 hover:bg-sky-500/25 disabled:opacity-50"
                                                                                        title={artifactPath}
                                                                                    >
                                                                                        {artifactOpenBusy === artifactPath ? "열기..." : `열기 ${artifactPathLabel(artifactPath)}`}
                                                                                    </button>
                                                                                ))}
                                                                            </div>
                                                                        )}
                                                                    </div>
                                                                );
                                                            })}
                                                        </div>
                                                    )}
                                                </div>
                                            ))}
                                        </div>
                                    )}
                                    {taskRunArtifacts.length > 0 && (
                                        <div className="mb-2 rounded border border-indigo-400/20 bg-indigo-500/10 px-2 py-2">
                                            <div className="flex items-center justify-between gap-2 mb-1">
                                                <div className="text-[11px] uppercase tracking-wider text-indigo-200 font-semibold">
                                                    Run Artifacts ({taskRunArtifacts.length})
                                                </div>
                                                <div className="flex items-center gap-1.5">
                                                    <select
                                                        value={artifactTypeFilter}
                                                        onChange={(e) => setArtifactTypeFilter(e.target.value)}
                                                        className="text-[10px] rounded border border-white/20 bg-black/30 px-2 py-0.5 text-gray-200"
                                                        title="아티팩트 타입 필터"
                                                    >
                                                        {artifactTypeOptions.map((option) => (
                                                            <option key={`artifact-filter-${option}`} value={option}>
                                                                {option === "all" ? "all types" : option}
                                                            </option>
                                                        ))}
                                                    </select>
                                                    <button
                                                        onClick={() => setArtifactFailedOnly((prev) => !prev)}
                                                        className={`text-[10px] px-2 py-0.5 rounded border ${
                                                            artifactFailedOnly
                                                                ? "border-rose-400/50 bg-rose-500/20 text-rose-100"
                                                                : "border-white/20 bg-black/30 text-gray-300"
                                                        }`}
                                                        title="실패한 assertion 키와 연결된 artifact만 보기"
                                                    >
                                                        failed only {artifactFailedOnly ? "ON" : "OFF"}
                                                    </button>
                                                    <select
                                                        value={artifactSortMode}
                                                        onChange={(e) =>
                                                            setArtifactSortMode(
                                                                e.target.value as ArtifactSortMode
                                                            )
                                                        }
                                                        className="text-[10px] rounded border border-white/20 bg-black/30 px-2 py-0.5 text-gray-200"
                                                        title="아티팩트 정렬"
                                                    >
                                                        <option value="failed_first">failed first</option>
                                                        <option value="newest">newest</option>
                                                        <option value="key">key</option>
                                                    </select>
                                                    <input
                                                        value={artifactSearchQuery}
                                                        onChange={(e) =>
                                                            setArtifactSearchQuery(e.target.value)
                                                        }
                                                        placeholder="search key/value"
                                                        className="text-[10px] rounded border border-white/20 bg-black/30 px-2 py-0.5 text-gray-200 w-28"
                                                    />
                                                </div>
                                            </div>
                                            <div className="space-y-2">
                                                {artifactGroups.length === 0 && (
                                                    <div className="text-[11px] text-gray-400 rounded border border-white/10 bg-black/20 px-2 py-1.5">
                                                        필터 조건에 맞는 artifact가 없습니다.
                                                    </div>
                                                )}
                                                {artifactGroups.map((group) => (
                                                    <div
                                                        key={`artifact-group-${group.type}`}
                                                        className="rounded border border-white/10 bg-black/20 px-2 py-1.5"
                                                    >
                                                        <div className="text-[11px] font-semibold text-indigo-100 mb-1">
                                                            {group.type} ({group.items.length})
                                                        </div>
                                                        <div className="space-y-1">
                                                            {group.items.slice(0, 6).map((artifact) => (
                                                                <div
                                                                    key={`artifact-${artifact.id}`}
                                                                    className="text-[11px] text-gray-200"
                                                                >
                                                                    <div className="flex items-center justify-between gap-2">
                                                                        <div className="font-mono text-[10px] text-indigo-100">
                                                                            {artifact.artifact_key}
                                                                            {failedArtifactKeys.has(artifact.artifact_key) && (
                                                                                <span className="ml-1 text-rose-300">• failed</span>
                                                                            )}
                                                                            {pinnedArtifactKeys.has(artifact.artifact_key) && (
                                                                                <span className="ml-1 text-amber-300">• pinned</span>
                                                                            )}
                                                                        </div>
                                                                        <div className="flex items-center gap-1">
                                                                            <button
                                                                                onClick={() =>
                                                                                    togglePinArtifactKey(
                                                                                        artifact.artifact_key
                                                                                    )
                                                                                }
                                                                                className={`text-[10px] px-2 py-0.5 rounded border ${
                                                                                    pinnedArtifactKeys.has(
                                                                                        artifact.artifact_key
                                                                                    )
                                                                                        ? "border-amber-300/45 bg-amber-400/20 text-amber-100"
                                                                                        : "border-white/20 bg-black/20 text-gray-300"
                                                                                }`}
                                                                            >
                                                                                pin
                                                                            </button>
                                                                            <button
                                                                                onClick={() => void copyArtifactPayload(artifact)}
                                                                                className="text-[10px] px-2 py-0.5 rounded border border-indigo-300/35 bg-indigo-400/20 text-indigo-100 hover:bg-indigo-400/30"
                                                                            >
                                                                                copy
                                                                            </button>
                                                                        </div>
                                                                    </div>
                                                                    <div>
                                                                        value={compactEvidence(artifact.value)}
                                                                    </div>
                                                                    {artifact.metadata && (
                                                                        <div className="text-gray-400">
                                                                            metadata={compactMetadata(artifact.metadata)}
                                                                        </div>
                                                                    )}
                                                                </div>
                                                            ))}
                                                            {group.items.length > 6 && (
                                                                <div className="text-[10px] text-gray-400">
                                                                    ... {group.items.length - 6} more
                                                                </div>
                                                            )}
                                                        </div>
                                                    </div>
                                                ))}
                                            </div>
                                        </div>
                                    )}
                                    {failedAssertions.length > 0 && (
                                        <div className="text-xs rounded border border-rose-500/30 bg-rose-500/10 text-rose-200 px-2 py-1.5">
                                            <div className="font-semibold mb-1">Failed Assertions ({failedAssertions.length})</div>
                                            <div className="space-y-1">
                                                {failedAssertions.slice(0, 4).map((a) => (
                                                    <div key={`assert-${a.id}`} className="opacity-90">
                                                        {a.stage_name}.{a.assertion_key}: expected={a.expected} actual={a.actual}
                                                    </div>
                                                ))}
                                            </div>
                                        </div>
                                    )}
                                </div>
                            )}

                            {pendingApproval && (
                                <div className="px-4 py-3 border-b border-white/5 bg-[#1b1b1b]">
                                    <div className="text-[11px] uppercase tracking-wider text-amber-400 font-semibold">
                                        Approval Required
                                    </div>
                                    <div className="text-sm text-gray-200 mt-1">
                                        Action: <span className="font-mono">{pendingApproval.action}</span>
                                    </div>
                                    <div className="text-xs text-gray-400">
                                        Risk: {pendingApproval.riskLevel} · Policy: {pendingApproval.policy}
                                    </div>
                                    <div className="text-xs text-gray-500 mt-1">
                                        {pendingApproval.message}
                                    </div>
                                    <div className="mt-3 flex flex-wrap gap-2">
                                        <button
                                            disabled={approvalBusy}
                                            onClick={() => handleApprovalDecision("allow_once")}
                                            className="text-xs px-3 py-1.5 rounded bg-emerald-500/20 text-emerald-200 border border-emerald-500/40 hover:bg-emerald-500/30 disabled:opacity-50"
                                        >
                                            Approve once
                                        </button>
                                        <button
                                            disabled={approvalBusy}
                                            onClick={() => handleApprovalDecision("allow_always")}
                                            className="text-xs px-3 py-1.5 rounded bg-blue-500/20 text-blue-200 border border-blue-500/40 hover:bg-blue-500/30 disabled:opacity-50"
                                        >
                                            Allow always
                                        </button>
                                        <button
                                            disabled={approvalBusy}
                                            onClick={() => handleApprovalDecision("deny")}
                                            className="text-xs px-3 py-1.5 rounded bg-rose-500/20 text-rose-200 border border-rose-500/40 hover:bg-rose-500/30 disabled:opacity-50"
                                        >
                                            Deny
                                        </button>
                                    </div>
                                </div>
                            )}

                            {lastStatus === "manual_required" && lastPlanId && !pendingApproval && (
                                <div className="px-4 py-3 border-b border-white/5 bg-[#171717]">
                                    <div className="text-[11px] uppercase tracking-wider text-sky-400 font-semibold">
                                        Manual Step Needed
                                    </div>
                                    <div className="text-xs text-gray-400 mt-1">
                                        브라우저에서 수동 작업을 완료한 뒤 Resume을 눌러 다음 단계로 진행하세요.
                                    </div>
                                    <div className="mt-3 space-y-2 text-xs text-gray-300">
                                        <label className="flex items-center gap-2">
                                            <input
                                                type="checkbox"
                                                checked={manualChecklist.focusReady}
                                                onChange={(e) =>
                                                    setManualChecklist((prev) => ({
                                                        ...prev,
                                                        focusReady: e.target.checked,
                                                    }))
                                                }
                                            />
                                            전면 앱을 작업 대상(브라우저/필요 앱)으로 복구함
                                        </label>
                                        <label className="flex items-center gap-2">
                                            <input
                                                type="checkbox"
                                                checked={manualChecklist.manualStepDone}
                                                onChange={(e) =>
                                                    setManualChecklist((prev) => ({
                                                        ...prev,
                                                        manualStepDone: e.target.checked,
                                                    }))
                                                }
                                            />
                                            수동 단계 입력/선택을 완료함
                                        </label>
                                        <label className="flex items-center gap-2">
                                            <input
                                                type="checkbox"
                                                checked={manualChecklist.handsOffReady}
                                                onChange={(e) =>
                                                    setManualChecklist((prev) => ({
                                                        ...prev,
                                                        handsOffReady: e.target.checked,
                                                    }))
                                                }
                                            />
                                            Resume 이후 키보드/마우스 간섭 없이 대기 가능
                                        </label>
                                    </div>
                                    <div className="mt-3 flex flex-wrap gap-2">
                                        <button
                                            disabled={
                                                loading ||
                                                !manualChecklist.focusReady ||
                                                !manualChecklist.manualStepDone ||
                                                !manualChecklist.handsOffReady
                                            }
                                            onClick={handleResume}
                                            className="text-xs px-3 py-1.5 rounded bg-sky-500/20 text-sky-200 border border-sky-500/40 hover:bg-sky-500/30 disabled:opacity-50"
                                        >
                                            Resume
                                        </button>
                                        {firstFailedArtifactPath && (
                                            <button
                                                disabled={loading || artifactOpenBusy === firstFailedArtifactPath}
                                                onClick={() => void handleGuidedRecovery()}
                                                className="text-xs px-3 py-1.5 rounded bg-amber-500/20 text-amber-200 border border-amber-500/40 hover:bg-amber-500/30 disabled:opacity-50"
                                                title={firstFailedArtifactPath}
                                            >
                                                증거 열고 Resume
                                            </button>
                                        )}
                                    </div>
                                </div>
                            )}

                            <div ref={scrollRef} className="launcher-feed max-h-[240px] overflow-y-auto p-3 space-y-2">
                                <AnimatePresence>
                                    {results.map((res, i) => {
                                        const isSelected = navigableItems.findIndex(x => x.id === `res-${i}`) === selectedIndex;
                                        return (
                                            <motion.div
                                                key={i}
                                                initial={{ opacity: 0, y: 10 }}
                                                animate={{ opacity: 1, y: 0 }}
                                                className={`launcher-feed-item p-3 rounded-lg text-gray-200 text-sm leading-relaxed transition-colors relative group ${isSelected ? 'bg-white/10' : 'bg-[#232323]'}`}
                                            >
                                                <ReactMarkdown components={markdownComponents}>
                                                    {res.content}
                                                </ReactMarkdown>
                                                <button
                                                    onClick={() => handlePin(res.content)}
                                                    className="absolute top-2 right-2 p-1.5 rounded-md text-gray-400 hover:text-white hover:bg-white/10 opacity-0 group-hover:opacity-100 transition-all"
                                                    title="Pin to Widget"
                                                >
                                                    <Pin className="w-4 h-4" />
                                                </button>
                                            </motion.div>
                                        )
                                    })}
                                </AnimatePresence>

                                {suggestionRecs.length > 0 && (
                                    <div className="pt-1">
                                        <div className="px-1 py-1 text-[11px] font-semibold text-gray-500 uppercase tracking-wider mb-1">
                                            Suggestions
                                        </div>
                                        {suggestionRecs.map((rec, idx) => {
                                            const isSel = navigableItems[selectedIndex]?.id === `rec-${rec.id}`;
                                            const recWorkflowUrl = resolveRecommendationWorkflowUrl(rec);
                                            const uiProvision = provisioningUiByRecId[rec.id];
                                            const statusLabel = formatRecommendationStatusLabel(rec, uiProvision);
                                            const isProvisioning =
                                                uiProvision?.phase === "provisioning" ||
                                                rec.workflow_id?.startsWith("provisioning:") ||
                                                uiProvision?.detail === "requested" ||
                                                uiProvision?.detail === "created";
                                            const canRetryProvision =
                                                uiProvision?.phase === "failed" || rec.status === "failed";
                                            const canApprove = rec.status === "pending" || canRetryProvision;
                                            const recN8nTarget =
                                                recWorkflowUrl ||
                                                (isProvisioning || canRetryProvision ? N8N_EDITOR_BASE_URL : null);
                                            return (
                                                <div
                                                    key={rec.id}
                                                    className={`launcher-suggestion-row group flex items-center justify-between px-3 py-2 rounded-md cursor-pointer transition-all mb-1 ${isSel ? 'bg-blue-500/20 border border-blue-500/30' : 'hover:bg-white/5 border border-transparent'}`}
                                                    onClick={() => {
                                                        const navIndex = navigableItems.findIndex(x => x.id === `rec-${rec.id}`);
                                                        if (navIndex >= 0) {
                                                            setSelectedIndex(navIndex);
                                                        } else {
                                                            setSelectedIndex(idx);
                                                        }
                                                    }}
                                                >
                                                    <div className="flex items-center gap-3">
                                                        <div className={`w-8 h-8 rounded flex items-center justify-center ${isSel ? 'bg-blue-500 text-white' : 'bg-white/10 text-gray-400'}`}>
                                                            <Zap className="w-4 h-4" />
                                                        </div>
                                                        <div>
                                                            <div className={`text-sm font-medium ${isSel ? 'text-blue-100' : 'text-gray-200'}`}>
                                                                {rec.title}
                                                            </div>
                                                            <div className="text-xs text-gray-500 line-clamp-1">
                                                                {rec.summary}
                                                            </div>
                                                            <div className="mt-1 flex items-center gap-1.5">
                                                                <span
                                                                    className={`text-[10px] px-2 py-0.5 rounded-full border ${recommendationStatusToneClass(rec, uiProvision)}`}
                                                                >
                                                                    {statusLabel}
                                                                </span>
                                                                {uiProvision?.updatedAt ? (
                                                                    <span className="text-[10px] text-gray-500">
                                                                        {formatProvisionUpdatedAt(uiProvision.updatedAt)}
                                                                    </span>
                                                                ) : null}
                                                            </div>
                                                            {uiProvision?.opId != null && (
                                                                <div className="mt-1 flex items-center gap-1.5">
                                                                    <span className="text-[10px] text-gray-400">
                                                                        op_id: {uiProvision.opId}
                                                                    </span>
                                                                    <button
                                                                        onClick={(e) => {
                                                                            e.stopPropagation();
                                                                            void copyTextValue(String(uiProvision.opId), "provision_op_id");
                                                                        }}
                                                                        className="text-[10px] px-1.5 py-0.5 rounded border border-white/15 text-gray-300 hover:bg-white/10"
                                                                    >
                                                                        복사
                                                                    </button>
                                                                </div>
                                                            )}
                                                            {(approveErrors[rec.id] || uiProvision?.phase === "failed") && (
                                                                <div className="mt-1 text-[10px] text-rose-300">
                                                                    {approveErrors[rec.id] || uiProvision?.detail}
                                                                </div>
                                                            )}
                                                        </div>
                                                    </div>

                                                    <div className="flex items-center gap-2">
                                                        <button
                                                            onClick={(e) => {
                                                                e.stopPropagation();
                                                                handlePin(rec.summary, rec.title);
                                                            }}
                                                            className="p-1.5 rounded-md text-gray-400 hover:text-white hover:bg-white/10 opacity-0 group-hover:opacity-100 transition-all"
                                                            title="Pin to Widget"
                                                        >
                                                            <Pin className="w-3 h-3" />
                                                        </button>

                                                        {recN8nTarget && (
                                                            <button
                                                                onClick={async (e) => {
                                                                    e.stopPropagation();
                                                                    const busyKey = `rec:${rec.id}`;
                                                                    setN8nOpenBusyKey(busyKey);
                                                                    try {
                                                                        await openExternalTarget(recN8nTarget);
                                                                        setResults([
                                                                            {
                                                                                type: "response",
                                                                                content: [
                                                                                    "**n8n 편집기 열기**",
                                                                                    `- recommendation_id: \`${rec.id}\``,
                                                                                    rec.workflow_id ? `- workflow_id: \`${rec.workflow_id}\`` : "",
                                                                                    `- URL: ${recN8nTarget}`,
                                                                                ]
                                                                                    .filter(Boolean)
                                                                                    .join("\n"),
                                                                            },
                                                                        ]);
                                                                        setShowDetailPanel(true);
                                                                        removeWatchRecommendation(rec.id);
                                                                    } catch (openError) {
                                                                        const openMsg =
                                                                            openError instanceof Error ? openError.message : String(openError);
                                                                        setApproveErrors((prev) => ({
                                                                            ...prev,
                                                                            [rec.id]: `n8n 열기 실패: ${openMsg}`,
                                                                        }));
                                                                    } finally {
                                                                        setN8nOpenBusyKey(null);
                                                                    }
                                                                }}
                                                                disabled={n8nOpenBusyKey === `rec:${rec.id}`}
                                                                className={`text-xs px-2 py-1 rounded transition-colors border ${isSel
                                                                    ? 'bg-sky-500 text-white border-sky-400'
                                                                    : 'text-sky-200 bg-sky-500/20 border-sky-400/30 hover:bg-sky-500/30'
                                                                    } ${n8nOpenBusyKey === `rec:${rec.id}` ? 'opacity-60 cursor-wait' : ''}`}
                                                            >
                                                                {n8nOpenBusyKey === `rec:${rec.id}`
                                                                    ? '열기…'
                                                                    : recWorkflowUrl
                                                                    ? 'n8n'
                                                                    : 'n8n 홈'}
                                                            </button>
                                                        )}

                                                        <button
                                                            onClick={(e) => {
                                                                e.stopPropagation();
                                                                if (!canApprove) return;
                                                                handleApprove(rec.id);
                                                            }}
                                                            disabled={approvingIds.has(rec.id) || !canApprove}
                                                            className={`text-xs px-3 py-1.5 rounded transition-colors border ${isSel
                                                                ? 'bg-blue-500 text-white border-blue-400'
                                                                : 'text-gray-200 bg-white/10 border-white/10 hover:bg-white/20'
                                                                } ${(approvingIds.has(rec.id) || !canApprove) ? 'opacity-60 cursor-not-allowed' : ''}`}
                                                        >
                                                            {approvingIds.has(rec.id)
                                                                ? 'Approving…'
                                                                : isProvisioning
                                                                    ? 'Provisioning…'
                                                                    : !canApprove
                                                                    ? 'Approved'
                                                                    : canRetryProvision || approveErrors[rec.id]
                                                                    ? 'Retry'
                                                                    : 'Approve'}
                                                        </button>

                                                        <div className="text-[10px] text-gray-500 bg-white/5 px-2 py-1 rounded">
                                                            Enter
                                                        </div>
                                                    </div>
                                                </div>
                                            )
                                        })}
                                    </div>
                                )}

                                {results.length === 0 && suggestionRecs.length === 0 && !pendingApproval && (
                                    <div className="p-6 text-center text-gray-500">
                                        <Terminal className="w-10 h-10 mx-auto mb-2 opacity-20" />
                                        <p className="text-sm">결과가 준비되면 여기에 표시됩니다.</p>
                                    </div>
                                )}
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>
            </motion.div>
        </div>
    );
}
