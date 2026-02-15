import axios from "axios";
import {
    SystemStatusSchema,
    RoutineSchema,
    LogEntrySchema,
    RecommendationSchema,
    RecommendationMetricsSchema,
    ExecApprovalSchema,
    ExecAllowlistSchema,
    ExecResultSchema,
    RoutineRunSchema,
    QualityScoreRecordSchema,
    ConsistencyCheckSchema,
    SemanticVerificationSchema,
    PerformanceVerificationSchema,
    VisualVerifySchema,
    RuntimeVerifySchema,
    ReleaseBaselineSchema,
    ReleaseGateSchema,
    VerificationRunSchema,
    AgentIntentResponseSchema,
    AgentPlanResponseSchema,
    AgentExecuteResponseSchema,
    AgentVerifyResponseSchema,
    AgentApproveResponseSchema,
    AgentPreflightResponseSchema,
    AgentPreflightFixResponseSchema,
    AgentRecoveryEventResponseSchema,
    ApprovalPolicySchema,
    NLRunMetricsSchema,
    NLRunSchema,
    TaskRunSchema,
    TaskStageRunSchema,
    TaskStageAssertionSchema,
    TaskRunArtifactSchema,
    ContextSelectionSchema,
    ProjectScanSchema,
    JudgmentSchema,
    type SystemStatus,
    type Routine,
    type LogEntry,
    type Recommendation,
    type RecommendationMetrics,
    type ExecApproval,
    type ExecAllowlistEntry,
    type ExecResult,
    type RoutineRun,
    type QualityScoreRecord,
    type ConsistencyCheck,
    type SemanticVerification,
    type PerformanceVerification,
    type VisualVerifyResult,
    type RuntimeVerifyResult,
    type ReleaseBaseline,
    type ReleaseGate,
    type VerificationRun,
    type AgentIntentResponse,
    type AgentPlanResponse,
    type ExecutionProfile,
    type AgentExecuteResponse,
    type AgentVerifyResponse,
    type AgentApproveResponse,
    type AgentPreflightResponse,
    type AgentPreflightFixResponse,
    type AgentRecoveryEventResponse,
    type ApprovalPolicy,
    type NLRunMetrics,
    type NLRun,
    type TaskRun,
    type TaskStageRun,
    type TaskStageAssertion,
    type TaskRunArtifact,
    type ContextSelection,
    type ProjectScan,
    type Judgment,
    type QualityScore,
} from "./types";
import { z } from "zod";

export const API_BASE_URL =
    import.meta.env.VITE_API_BASE_URL?.replace(/\/$/, "") ??
    "http://localhost:5680/api";

const api = axios.create({
    baseURL: API_BASE_URL,
    timeout: 5000,
});

// Paranoid: Validate all responses with Zod
export async function fetchSystemStatus(): Promise<SystemStatus> {
    const { data } = await api.get("/status");
    return SystemStatusSchema.parse(data);
}

export async function fetchLogs(): Promise<LogEntry[]> {
    const { data } = await api.get("/logs");
    return z.array(LogEntrySchema).parse(data);
}

export async function fetchRoutines(): Promise<Routine[]> {
    const { data } = await api.get("/routines");
    return z.array(RoutineSchema).parse(data);
}

export async function createRoutine(name: string, cron: string, prompt: string): Promise<void> {
    await api.post("/routines", { name, cron_expression: cron, prompt });
}

export async function toggleRoutine(id: number, enabled: boolean): Promise<void> {
    await api.patch(`/routines/${id}`, { enabled });
}

// Workflows / Recommendations
export async function fetchRecommendations(): Promise<Recommendation[]> {
    const { data } = await api.get("/recommendations?status=all");
    return z.array(RecommendationSchema).parse(data);
}

export async function approveRecommendation(id: number): Promise<void> {
    await api.post(`/recommendations/${id}/approve`, undefined, { timeout: 20000 });
}

export async function rejectRecommendation(id: number): Promise<void> {
    await api.post(`/recommendations/${id}/reject`);
}

export async function laterRecommendation(id: number): Promise<void> {
    await api.post(`/recommendations/${id}/later`);
}

export async function restoreRecommendation(id: number): Promise<void> {
    await api.post(`/recommendations/${id}/restore`);
}

export async function fetchRecommendationMetrics(): Promise<RecommendationMetrics> {
    const { data } = await api.get("/recommendations/metrics");
    return RecommendationMetricsSchema.parse(data);
}

export async function fetchExecApprovals(status: string = "pending"): Promise<ExecApproval[]> {
    const { data } = await api.get(`/exec-approvals?status=${encodeURIComponent(status)}`);
    return z.array(ExecApprovalSchema).parse(data);
}

export async function approveExecApproval(id: string, resolvedBy?: string): Promise<void> {
    await api.post(`/exec-approvals/${id}/approve`, resolvedBy ? { resolved_by: resolvedBy } : undefined);
}

export async function rejectExecApproval(id: string, resolvedBy?: string): Promise<void> {
    await api.post(`/exec-approvals/${id}/reject`, resolvedBy ? { resolved_by: resolvedBy } : undefined);
}

export async function fetchExecAllowlist(limit: number = 100): Promise<ExecAllowlistEntry[]> {
    const { data } = await api.get(`/exec-allowlist?limit=${limit}`);
    return z.array(ExecAllowlistSchema).parse(data);
}

export async function addExecAllowlist(pattern: string, cwd?: string): Promise<void> {
    await api.post(`/exec-allowlist`, { pattern, cwd });
}

export async function removeExecAllowlist(id: number): Promise<void> {
    await api.delete(`/exec-allowlist/${id}`);
}

export async function fetchExecResults(limit: number = 100, status?: string): Promise<ExecResult[]> {
    const query = new URLSearchParams();
    query.set("limit", String(limit));
    if (status) query.set("status", status);
    const { data } = await api.get(`/exec-results?${query.toString()}`);
    return z.array(ExecResultSchema).parse(data);
}

export async function runExecResultsGuard(maxAgeSecs?: number, limit?: number): Promise<{ ok: boolean; scanned: number; timed_out: number; warnings: string[]; template: string }> {
    const payload: Record<string, number> = {};
    if (maxAgeSecs !== undefined) payload.max_age_secs = maxAgeSecs;
    if (limit !== undefined) payload.limit = limit;
    const { data } = await api.post(`/exec-results/guard`, payload);
    return data;
}

export async function fetchRoutineRuns(limit: number = 20): Promise<RoutineRun[]> {
    const { data } = await api.get(`/routine-runs?limit=${limit}`);
    return z.array(RoutineRunSchema).parse(data);
}

export async function fetchLatestQualityScore(): Promise<QualityScoreRecord | null> {
    const { data } = await api.get("/quality/latest");
    if (!data) return null;
    return QualityScoreRecordSchema.parse(data);
}

export async function calculateQualityScore(): Promise<QualityScoreRecord> {
    const { data } = await api.post("/quality/score");
    return QualityScoreRecordSchema.parse(data);
}

export async function fetchConsistencyCheck(): Promise<ConsistencyCheck> {
    const { data } = await api.post("/verify/consistency", {});
    return ConsistencyCheckSchema.parse(data);
}

export async function fetchSemanticVerification(): Promise<SemanticVerification> {
    const { data } = await api.post("/verify/semantic", {});
    return SemanticVerificationSchema.parse(data);
}

export type RuntimeVerifyOptions = {
    workdir?: string;
    run_backend?: boolean;
    run_frontend?: boolean;
    run_e2e?: boolean;
    run_build_checks?: boolean;
    backend_port?: number;
    frontend_port?: number;
    backend_health_path?: string;
};

export async function runRuntimeVerification(options: RuntimeVerifyOptions = {}): Promise<RuntimeVerifyResult> {
    const { data } = await api.post("/verify/runtime", options);
    return RuntimeVerifySchema.parse(data);
}

export type PerformanceVerifyOptions = {
    workdir?: string;
    max_files?: number;
};

export async function runPerformanceVerification(options: PerformanceVerifyOptions = {}): Promise<PerformanceVerification> {
    const { data } = await api.post("/verify/performance", options);
    return PerformanceVerificationSchema.parse(data);
}

export async function runVisualVerification(prompts: string[]): Promise<VisualVerifyResult> {
    const { data } = await api.post("/verify/visual", { prompts });
    return VisualVerifySchema.parse(data);
}

// Beta Features
export async function fetchSelectionContext(): Promise<ContextSelection> {
    const { data } = await api.get("/context/selection");
    return ContextSelectionSchema.parse(data);
}

export async function scanProject(maxFiles?: number, workdir?: string): Promise<ProjectScan> {
    const query = new URLSearchParams();
    if (maxFiles) query.set("max_files", String(maxFiles));
    if (workdir) query.set("workdir", workdir);
    const { data } = await api.get(`/project/scan?${query.toString()}`);
    return ProjectScanSchema.parse(data);
}

export async function runJudgment(
    workdir?: string,
    maxFiles?: number,
    runtime?: RuntimeVerifyResult,
    quality?: QualityScore,
    semantic?: SemanticVerification,
    performance?: PerformanceVerification,
): Promise<Judgment> {
    const payload = {
        workdir,
        max_files: maxFiles,
        runtime,
        quality,
        semantic,
        performance,
    };
    const { data } = await api.post("/judgment", payload);
    return JudgmentSchema.parse(data);
}

export type ReleaseGateOverrides = {
    perf_regression_pct?: number;
    quality_drop?: number;
};

export async function fetchReleaseGate(overrides?: ReleaseGateOverrides): Promise<ReleaseGate> {
    const payload: Record<string, number> = {};
    if (overrides?.perf_regression_pct !== undefined) {
        payload.perf_regression_pct = overrides.perf_regression_pct;
    }
    if (overrides?.quality_drop !== undefined) {
        payload.quality_drop = overrides.quality_drop;
    }
    const { data } = await api.post("/release/gate", payload);
    return ReleaseGateSchema.parse(data);
}

export async function setReleaseBaseline(options: PerformanceVerifyOptions = {}): Promise<ReleaseBaseline> {
    const { data } = await api.post("/release/baseline", options);
    return ReleaseBaselineSchema.parse(data);
}

export async function fetchVerificationRuns(limit: number = 20): Promise<VerificationRun[]> {
    const { data } = await api.get(`/verify/runs?limit=${limit}`);
    return z.array(VerificationRunSchema).parse(data);
}

export async function agentIntent(text: string): Promise<AgentIntentResponse> {
    const { data } = await api.post("/agent/intent", { text });
    return AgentIntentResponseSchema.parse(data);
}

export async function agentPlan(
    sessionId: string,
    slots?: Record<string, string>
): Promise<AgentPlanResponse> {
    const { data } = await api.post("/agent/plan", { session_id: sessionId, slots });
    return AgentPlanResponseSchema.parse(data);
}

export async function agentExecute(
    planId: string,
    profile?: ExecutionProfile
): Promise<AgentExecuteResponse> {
    const payload: Record<string, unknown> = { plan_id: planId };
    if (profile) {
        payload.profile = profile;
    }
    const { data } = await api.post("/agent/execute", payload);
    return AgentExecuteResponseSchema.parse(data);
}

export async function agentVerify(planId: string): Promise<AgentVerifyResponse> {
    const { data } = await api.post("/agent/verify", { plan_id: planId });
    return AgentVerifyResponseSchema.parse(data);
}

export async function agentApprove(
    planId: string,
    action: string,
    decision?: string
): Promise<AgentApproveResponse> {
    const { data } = await api.post("/agent/approve", { plan_id: planId, action, decision });
    return AgentApproveResponseSchema.parse(data);
}

export async function fetchAgentPreflight(): Promise<AgentPreflightResponse> {
    const { data } = await api.get("/agent/preflight");
    return AgentPreflightResponseSchema.parse(data);
}

export type AgentPreflightFixOptions = {
    run_id?: string | null;
    stage_name?: string;
    assertion_key?: string;
};

export async function runAgentPreflightFix(
    action: string,
    options?: AgentPreflightFixOptions
): Promise<AgentPreflightFixResponse> {
    const payload: Record<string, unknown> = { action };
    if (options?.run_id) payload.run_id = options.run_id;
    if (options?.stage_name) payload.stage_name = options.stage_name;
    if (options?.assertion_key) payload.assertion_key = options.assertion_key;
    const { data } = await api.post("/agent/preflight/fix", payload);
    return AgentPreflightFixResponseSchema.parse(data);
}

export type AgentRecoveryEventRequest = {
    run_id: string;
    action_key: string;
    status: string;
    details?: string;
    stage_name?: string;
    expected?: string;
    actual?: string;
};

export async function recordAgentRecoveryEvent(
    payload: AgentRecoveryEventRequest
): Promise<AgentRecoveryEventResponse> {
    const { data } = await api.post("/agent/recovery-event", payload);
    return AgentRecoveryEventResponseSchema.parse(data);
}

export async function fetchNlRuns(limit: number = 20): Promise<NLRun[]> {
    const { data } = await api.get(`/agent/nl-runs?limit=${limit}`);
    return z.array(NLRunSchema).parse(data);
}

export async function fetchNlRunMetrics(limit: number = 50): Promise<NLRunMetrics> {
    const { data } = await api.get(`/agent/nl-metrics?limit=${limit}`);
    return NLRunMetricsSchema.parse(data);
}

export async function fetchTaskRuns(limit: number = 50, status?: string): Promise<TaskRun[]> {
    const query = new URLSearchParams();
    query.set("limit", String(limit));
    if (status && status.trim()) query.set("status", status.trim());
    const { data } = await api.get(`/agent/task-runs?${query.toString()}`);
    return z.array(TaskRunSchema).parse(data);
}

export async function fetchTaskRun(runId: string): Promise<TaskRun> {
    const { data } = await api.get(`/agent/task-runs/${encodeURIComponent(runId)}`);
    return TaskRunSchema.parse(data);
}

export async function fetchTaskRunStages(runId: string): Promise<TaskStageRun[]> {
    const { data } = await api.get(`/agent/task-runs/${encodeURIComponent(runId)}/stages`);
    return z.array(TaskStageRunSchema).parse(data);
}

export async function fetchTaskRunAssertions(runId: string): Promise<TaskStageAssertion[]> {
    const { data } = await api.get(`/agent/task-runs/${encodeURIComponent(runId)}/assertions`);
    return z.array(TaskStageAssertionSchema).parse(data);
}

export async function fetchTaskRunArtifacts(runId: string): Promise<TaskRunArtifact[]> {
    const { data } = await api.get(`/agent/task-runs/${encodeURIComponent(runId)}/artifacts`);
    return z
        .object({
            artifacts: z.array(TaskRunArtifactSchema),
        })
        .parse(data).artifacts;
}

export async function fetchApprovalPolicies(limit: number = 20): Promise<ApprovalPolicy[]> {
    const { data } = await api.get(`/agent/approval-policies?limit=${limit}`);
    return z.array(ApprovalPolicySchema).parse(data);
}

export async function removeApprovalPolicy(policyKey: string): Promise<void> {
    await api.delete(`/agent/approval-policies/${encodeURIComponent(policyKey)}`);
}

export async function sendFeedback(
    goal: string,
    feedback: string,
    historySummary?: string
): Promise<{ action: string; new_goal?: string; message: string }> {
    const { data } = await api.post("/agent/feedback", {
        goal,
        feedback,
        history_summary: historySummary || undefined,
    });
    return data;
}

export async function executeGoal(goal: string): Promise<{ status: string; message: string }> {
    const { data } = await api.post("/agent/goal", { goal });
    return data;
}

export async function fetchCurrentGoal(): Promise<string> {
    const { data } = await api.get("/agent/goal/current");
    return typeof data?.goal === "string" ? data.goal : "";
}

export async function getHealth(): Promise<unknown> {
    const { data } = await api.get("/system/health");
    return data;
}

export async function analyzePatterns(): Promise<string[]> {
    const { data } = await api.post("/patterns/analyze");
    return z.array(z.string()).parse(data);
}

export async function sendChatMessage(message: string): Promise<{ response: string; command?: string }> {
    try {
        const { data } = await api.post("/chat", { message });
        return data;
    } catch (e) {
        if (axios.isAxiosError(e)) {
            console.error("Chat Error:", e.response?.data || e.message);
            return { response: "❌ Network or Server Error. Check console logs." };
        }
        return { response: "❌ Unknown Error" };
    }
}
