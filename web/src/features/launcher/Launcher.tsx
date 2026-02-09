import { useState, useEffect, useRef } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Search, Zap, Activity, Terminal, Pin } from "lucide-react"; // Added Pin icon
import { sendChatMessage, approveRecommendation, agentIntent, agentPlan, agentExecute, agentVerify, agentApprove } from "@/lib/api";
import { useRecommendations } from "@/lib/hooks";
import { emit } from "@tauri-apps/api/event"; // Added emit
import { getAllWindows, getCurrentWindow } from "@tauri-apps/api/window"; // Added getAllWindows
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
    const [results, setResults] = useState<LauncherResult[]>([]);
    const [loading, setLoading] = useState(false);
    const [selectedIndex, setSelectedIndex] = useState(0);
    const [successPulse, setSuccessPulse] = useState(false);
    const [shake, setShake] = useState(false);
    const [approvingIds, setApprovingIds] = useState<Set<number>>(new Set());
    const [approveErrors, setApproveErrors] = useState<Record<number, string>>({});
    const [approveCooldowns, setApproveCooldowns] = useState<Record<number, number>>({});
    const [pendingApproval, setPendingApproval] = useState<ApprovalContext | null>(null);
    const [approvalBusy, setApprovalBusy] = useState(false);
    const [lastPlanId, setLastPlanId] = useState<string | null>(null);
    const [lastStatus, setLastStatus] = useState<string | null>(null);
    const inputRef = useRef<HTMLInputElement>(null);
    const scrollRef = useRef<HTMLDivElement>(null);
    const { data: recs, refetch } = useRecommendations();

    // Auto-focus input on mount
    useEffect(() => {
        inputRef.current?.focus();
    }, []);

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
                title: title || 'Pinned from Steer'
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
    const navigableItems = [
        ...results.map((r, i) => ({ type: 'result', data: r, id: `res-${i}` })),
        ...pendingRecs.map(r => ({ type: 'recommendation', data: r, id: `rec-${r.id}` }))
    ];

    // Reset selection when items change
    useEffect(() => {
        setSelectedIndex(0);
    }, [results, pendingRecs.length]);

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

    const handleSend = async () => {
        if (!input.trim()) return;
        setLoading(true);
        setResults([]);
        setPendingApproval(null);

        // [Phase 6.3] Performance Test Command
        if (input.trim() === "test_perf") {
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
            const intentRes = await agentIntent(input);
            if (intentRes.missing_slots && intentRes.missing_slots.length > 0) {
                const followUp = intentRes.follow_up || "추가 정보가 필요합니다.";
                setResults([{
                    type: 'response',
                    content: `**추가 입력 필요**\n- Missing: ${intentRes.missing_slots.join(", ")}\n- ${followUp}`
                }]);
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
                setLoading(false);
                return;
            }
            const execRes = await agentExecute(planRes.plan_id);
            setLastStatus(execRes.status);
            const verifyRes = await agentVerify(planRes.plan_id);

            const summaryLines = [
                `**Intent**: ${intentRes.intent} (${Math.round(intentRes.confidence * 100)}%)`,
                `**Status**: ${execRes.status}`,
                `**Verify**: ${verifyRes.ok ? "ok" : "issues"}`,
                execRes.resume_from != null ? `**Next Step**: ${execRes.resume_from + 1}` : "",
            ];
            const logLines = execRes.logs?.slice(0, 10).map(line => `- ${line}`) ?? [];
            const verifyLines = verifyRes.issues?.length ? verifyRes.issues.map(i => `- ${i}`) : [];
            const manualLines = execRes.manual_steps?.length
                ? execRes.manual_steps.map(step => `- ${step}`)
                : [];

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
            try {
                const res = await sendChatMessage(input);
                setResults([{ type: 'response', content: res.response }]);
                setInput("");
                triggerSuccess();
            } catch {
                setResults([{ type: 'error', content: "Failed to reach agent." }]);
                triggerError();
            }
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

    const handleResume = async () => {
        if (!lastPlanId) return;
        setLoading(true);
        try {
            const execRes = await agentExecute(lastPlanId);
            setLastStatus(execRes.status);
            const verifyRes = await agentVerify(lastPlanId);
            const summaryLines = [
                `**Status**: ${execRes.status}`,
                `**Verify**: ${verifyRes.ok ? "ok" : "issues"}`,
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
            triggerError();
        } finally {
            setLoading(false);
        }
    };

    const handleApprovalDecision = async (decision: "allow_once" | "allow_always" | "deny") => {
        if (!pendingApproval) return;
        setApprovalBusy(true);
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
                triggerSuccess();
                return;
            }
            const execRes = await agentExecute(pendingApproval.planId);
            const verifyRes = await agentVerify(pendingApproval.planId);
            const summaryLines = [
                `**Status**: ${execRes.status}`,
                `**Verify**: ${verifyRes.ok ? "ok" : "issues"}`,
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
        try {
            await approveRecommendation(id);
            triggerSuccess();
        } catch (e) {
            console.error("Approve failed", e);
            triggerError();
            const raw = extractErrorMessage(e);
            setApproveErrors(prev => ({ ...prev, [id]: mapApproveError(raw) }));
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
            return "요청 시간이 초과됨. 잠시 후 재시도";
        }
        if (msg.includes("connection refused")) {
            return "코어 서버 연결 실패 (5680 실행 상태 확인)";
        }
        return raw;
    };

    // Keyboard Handler
    const handleKeyDown = async (e: React.KeyboardEvent) => {
        if (e.key === "ArrowDown") {
            e.preventDefault();
            setSelectedIndex(prev => (prev + 1) % navigableItems.length);
        } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setSelectedIndex(prev => (prev - 1 + navigableItems.length) % navigableItems.length);
        } else if (e.key === "Enter") {
            if (input.trim() && navigableItems.length === 0) {
                e.preventDefault();
                await handleSend();
                return;
            }

            if (navigableItems.length > 0) {
                const selected = navigableItems[selectedIndex];
                if (selected && selected.type === 'recommendation') {
                    e.preventDefault();
                    const rec = selected.data as { id: number; title: string; summary: string; status: string };
                    await handleApprove(rec.id);
                }
            } else if (input.trim()) {
                await handleSend();
            }
        }
    };

    const handleBackgroundClick = async (e: React.MouseEvent) => {
        if (e.target === e.currentTarget) {
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

    return (
        <div
            className="fixed inset-0 bg-transparent flex items-start justify-center pt-[20vh] p-8"
            onMouseDown={handleBackgroundClick}
        >
            <motion.div
                className={`w-full max-w-2xl bg-[#1e1e1e]/95 backdrop-blur-2xl rounded-2xl shadow-2xl overflow-hidden border transition-colors duration-500
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
                {/* Input Area */}
                <div className="flex items-center px-4 py-4 border-b border-white/5 bg-[#1e1e1e]">
                    <Search className="w-5 h-5 text-gray-400 mr-3" />
                    <input
                        ref={inputRef}
                        type="text"
                        className="flex-1 bg-transparent border-none outline-none text-lg text-white placeholder-gray-500 font-medium"
                        placeholder="무엇이든 부탁하세요 (Ask anything...)"
                        value={input}
                        onChange={(e) => setInput(e.target.value)}
                        onKeyDown={handleKeyDown}
                        autoFocus
                    />
                    {loading && <Activity className="w-5 h-5 text-blue-500 animate-spin" />}
                    {!loading && (
                        <button
                            onClick={handleSend}
                            disabled={!input.trim()}
                            className="ml-2 text-xs text-gray-200 bg-white/10 hover:bg-white/20 px-2 py-1 rounded disabled:opacity-50"
                        >
                            Send
                        </button>
                    )}
                </div>

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
                        <div className="mt-3">
                            <button
                                disabled={loading}
                                onClick={handleResume}
                                className="text-xs px-3 py-1.5 rounded bg-sky-500/20 text-sky-200 border border-sky-500/40 hover:bg-sky-500/30 disabled:opacity-50"
                            >
                                Resume
                            </button>
                        </div>
                    </div>
                )}

                {/* Content Area */}
                <div ref={scrollRef} className="bg-[#191919] min-h-[300px] max-h-[500px] overflow-y-auto">

                    {/* 1. Response View */}
                    <AnimatePresence>
                        {results.map((res, i) => {
                            const isSelected = navigableItems.findIndex(x => x.id === `res-${i}`) === selectedIndex;
                            return (
                                <motion.div
                                    key={i}
                                    initial={{ opacity: 0, y: 10 }}
                                    animate={{ opacity: 1, y: 0 }}
                                    className={`p-4 rounded-lg mb-2 text-gray-200 text-sm leading-relaxed transition-colors relative group ${isSelected ? 'bg-white/10' : 'bg-[#2a2a2a]'}`}
                                >
                                    {/* Content */}
                                    <ReactMarkdown components={markdownComponents}>
                                        {res.content}
                                    </ReactMarkdown>

                                    {/* Pin Button */}
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

                    {/* 2. Pending Approvals */}
                    {pendingRecs.length > 0 && (
                        <div className="p-2">
                            <div className="px-2 py-1 text-xs font-semibold text-gray-500 uppercase tracking-wider mb-1">
                                Suggestions
                            </div>
                            {pendingRecs.map((rec, idx) => {
                                const isSel = navigableItems[selectedIndex]?.id === `rec-${rec.id}`;
                                return (
                                    <div
                                        key={rec.id}
                                        className={`group flex items-center justify-between px-3 py-2 rounded-md cursor-pointer transition-all ${isSel ? 'bg-blue-500/20 border border-blue-500/30' : 'hover:bg-white/5 border border-transparent'}`}
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
                                                {approveErrors[rec.id] && (
                                                    <div className="mt-1 text-[10px] text-rose-300">
                                                        {approveErrors[rec.id]}
                                                    </div>
                                                )}
                                            </div>
                                        </div>

                                        <div className="flex items-center gap-2">
                                            {/* Pin Button for Recs */}
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

                                            <button
                                                onClick={(e) => {
                                                    e.stopPropagation();
                                                    handleApprove(rec.id);
                                                }}
                                                disabled={approvingIds.has(rec.id)}
                                                className={`text-xs px-3 py-1.5 rounded transition-colors border ${isSel
                                                    ? 'bg-blue-500 text-white border-blue-400'
                                                    : 'text-gray-200 bg-white/10 border-white/10 hover:bg-white/20'
                                                    } ${approvingIds.has(rec.id) ? 'opacity-60 cursor-wait' : ''}`}
                                            >
                                                {approvingIds.has(rec.id)
                                                    ? 'Approving…'
                                                    : approveErrors[rec.id]
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

                    {/* 3. Empty State */}
                    {results.length === 0 && pendingRecs.length === 0 && (
                        <div className="p-8 text-center text-gray-500">
                            <Terminal className="w-12 h-12 mx-auto mb-3 opacity-20" />
                            <p className="text-sm">Type a command or chat with your agent.</p>
                            <div className="mt-4 flex flex-wrap justify-center gap-2">
                                <span className="text-xs bg-white/5 px-2 py-1 rounded hover:bg-white/10 cursor-pointer">매일 아침 뉴스 요약</span>
                                <span className="text-xs bg-white/5 px-2 py-1 rounded hover:bg-white/10 cursor-pointer">화면 클릭해줘</span>
                                <span className="text-xs bg-white/5 px-2 py-1 rounded hover:bg-white/10 cursor-pointer">이 프로젝트 분석해줘</span>
                            </div>
                        </div>
                    )}
                </div>

                {/* Footer Status */}
                <div className="px-4 py-2 border-t border-white/5 bg-[#1e1e1e] flex items-center justify-between">
                    <div className="flex items-center gap-2">
                        <div className="w-2 h-2 rounded-full bg-green-500 shadow-lg shadow-green-500/50"></div>
                        <span className="text-xs text-gray-500">Engine Active</span>
                    </div>
                    <div className="flex gap-4 text-xs text-gray-600">
                        <span className="hover:text-gray-400 cursor-pointer">Settings</span>
                        <span className="hover:text-gray-400 cursor-pointer">Help</span>
                    </div>
                </div>
            </motion.div>
        </div>
    );
}
