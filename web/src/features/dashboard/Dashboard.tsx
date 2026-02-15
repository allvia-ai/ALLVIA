import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogTrigger } from "@/components/ui/dialog";
import { Activity, Cpu, HardDrive, RefreshCw, Lightbulb, ShieldCheck } from "lucide-react";
import { AuditLog } from "@/components/AuditLog";
import { useSystemStatus, useLogs, useRoutines, useRecommendations, useRecommendationMetrics, useQualityScore, useConsistencyCheck, useSemanticVerification, useReleaseGate, useVerificationRuns, useExecAllowlist, useExecResults, useExecApprovals, useRoutineRuns, useNlRuns, useNlRunMetrics, useApprovalPolicies } from "@/lib/hooks";
import { approveRecommendation, rejectRecommendation, laterRecommendation, restoreRecommendation, sendFeedback, fetchCurrentGoal, addExecAllowlist, removeExecAllowlist, runExecResultsGuard, runRuntimeVerification, runPerformanceVerification, runVisualVerification, setReleaseBaseline, fetchSelectionContext, scanProject, runJudgment, approveExecApproval, rejectExecApproval, analyzePatterns, createRoutine, toggleRoutine, calculateQualityScore, executeGoal, agentIntent, agentPlan, agentExecute, agentVerify, agentApprove, removeApprovalPolicy, fetchTaskRunStages, fetchTaskRunAssertions } from "@/lib/api";
import { format } from "date-fns";
import { motion } from "framer-motion";
import { useEffect, useState } from "react";
import type { ReleaseGateOverrides } from "@/lib/api";
import type {
    RuntimeVerifyResult,
    PerformanceVerification,
    VisualVerifyResult,
    ProjectScan,
    Judgment,
    ContextSelection,
    ExecutionProfile,
    TaskStageRun,
    TaskStageAssertion,
} from "@/lib/types";

const containerVariants = {
    hidden: { opacity: 0 },
    visible: { opacity: 1, transition: { staggerChildren: 0.1 } }
};

const cardVariants = {
    hidden: { opacity: 0, y: 20, scale: 0.95 },
    visible: { opacity: 1, y: 0, scale: 1, transition: { duration: 0.4 } }
};

export default function Dashboard() {
    // Keep the data stability fixes (isError ignored, placeholderData in hooks)
    const { data: status, isFetching, isError: statusError } = useSystemStatus();
    const { data: logs, isLoading: logsLoading, isError: logsError } = useLogs();
    const { data: verificationRuns } = useVerificationRuns(20);
    const { data: routines } = useRoutines();
    const { data: recMetrics } = useRecommendationMetrics();
    const { data: qualityScore } = useQualityScore();
    const [expandedRunId, setExpandedRunId] = useState<number | null>(null);
    const [runKindFilter, setRunKindFilter] = useState("all");
    const [runStatusFilter, setRunStatusFilter] = useState<"all" | "ok" | "fail">("all");

    const activeRoutinesCount = routines?.filter(r => r.enabled).length ?? 0;
    const isOffline = statusError || logsError;

    // Use stable values
    const cpuValue = status?.cpu_usage?.toFixed(1) ?? "0";
    const memoryUsed = ((status?.memory_used ?? 0) / 1024).toFixed(1);
    const memoryTotal = ((status?.memory_total ?? 16384) / 1024).toFixed(0);
    const approvalRate = recMetrics?.approval_rate?.toFixed(0) ?? "0";
    const lastRecTime = recMetrics?.last_created_at
        ? format(new Date(recMetrics.last_created_at), "HH:mm")
        : "—";

    const qualityValue = qualityScore?.score?.overall?.toFixed(1) ?? "—";
    const qualityLabel = qualityScore?.score?.recommendation ?? "pending";
    const qualityTime = qualityScore?.created_at
        ? format(new Date(qualityScore.created_at), "HH:mm")
        : "—";

    const runKinds = Array.from(
        new Set((verificationRuns ?? []).map((run) => run.kind))
    );
    const filteredRuns = (verificationRuns ?? [])
        .filter((run) => (runKindFilter === "all" ? true : run.kind === runKindFilter))
        .filter((run) => {
            if (runStatusFilter === "all") return true;
            return runStatusFilter === "ok" ? run.ok : !run.ok;
        });

    return (
        <div className="space-y-6">
            <motion.div
                className="flex items-center justify-between"
                initial={{ opacity: 0, y: -20 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ duration: 0.5 }}
            >
                <h2 className="text-3xl font-bold tracking-tight text-glow">Dashboard</h2>
                <div className="flex items-center gap-2">
                    {isFetching && (
                        <RefreshCw className="w-4 h-4 animate-spin text-muted-foreground" />
                    )}
                    <span className="relative flex h-3 w-3">
                        <span className={`animate-ping absolute inline-flex h-full w-full rounded-full ${isOffline ? "bg-red-400" : "bg-green-400"} opacity-75`}></span>
                        <span className={`relative inline-flex rounded-full h-3 w-3 ${isOffline ? "bg-red-500" : "bg-green-500"}`}></span>
                    </span>
                    <span className="text-sm text-muted-foreground font-mono">{isOffline ? "Offline" : "Live"}</span>
                </div>
            </motion.div>

            <motion.div
                className="max-w-4xl mx-auto space-y-6"
                variants={containerVariants}
                initial="hidden"
                animate="visible"
            >
                {isOffline && (
                    <motion.div variants={cardVariants}>
                        <Card className="border-amber-400/30 bg-amber-500/10">
                            <CardContent className="p-4 text-sm text-amber-200">
                                API unreachable. Check that the core server is running on localhost:5680.
                            </CardContent>
                        </Card>
                    </motion.div>
                )}
                <ControlCard />
                <NaturalLanguageAutomationCard />
                {/* <RecommendationsCard /> */}
                <motion.div variants={cardVariants} whileHover={{ scale: 1.02, y: -2 }} transition={{ type: "spring", stiffness: 300 }}>
                    <Card className="border-primary/20 bg-primary/5 h-full">
                        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                            <CardTitle className="text-sm font-medium">CPU Load</CardTitle>
                            <Cpu className="h-4 w-4 text-primary" />
                        </CardHeader>
                        <CardContent>
                            <div className="text-2xl font-bold" style={{ fontVariantNumeric: 'tabular-nums' }}>
                                {cpuValue}%
                            </div>
                            <p className="text-xs text-muted-foreground">Real-time usage</p>
                        </CardContent>
                    </Card>
                </motion.div>

                {/* Memory Card */}
                <motion.div variants={cardVariants} whileHover={{ scale: 1.02, y: -2 }} transition={{ type: "spring", stiffness: 300 }}>
                    <Card className="h-full">
                        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                            <CardTitle className="text-sm font-medium">Memory</CardTitle>
                            <HardDrive className="h-4 w-4 text-muted-foreground" />
                        </CardHeader>
                        <CardContent>
                            <div className="text-2xl font-bold" style={{ fontVariantNumeric: 'tabular-nums' }}>
                                {memoryUsed} GB
                            </div>
                            <p className="text-xs text-muted-foreground">
                                Used of {memoryTotal} GB
                            </p>
                        </CardContent>
                    </Card>
                </motion.div>

                {/* Active Routines Card */}
                <motion.div variants={cardVariants} whileHover={{ scale: 1.02, y: -2 }} transition={{ type: "spring", stiffness: 300 }}>
                    <Card className="h-full">
                        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                            <CardTitle className="text-sm font-medium">Active Routines</CardTitle>
                            <Activity className="h-4 w-4 text-muted-foreground" />
                        </CardHeader>
                        <CardContent>
                            <div className="text-2xl font-bold" style={{ fontVariantNumeric: 'tabular-nums' }}>
                                {activeRoutinesCount}
                            </div>
                            <p className="text-xs text-muted-foreground">Running perfectly</p>
                        </CardContent>
                    </Card>
                </motion.div>

                {/* Recommendations Card */}
                <motion.div variants={cardVariants} whileHover={{ scale: 1.02, y: -2 }} transition={{ type: "spring", stiffness: 300 }}>
                    <Card className="h-full">
                        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                            <CardTitle className="text-sm font-medium">Recommendations</CardTitle>
                            <Lightbulb className="h-4 w-4 text-muted-foreground" />
                        </CardHeader>
                        <CardContent>
                            <div className="text-2xl font-bold" style={{ fontVariantNumeric: 'tabular-nums' }}>
                                {recMetrics?.pending ?? 0}
                            </div>
                            <p className="text-xs text-muted-foreground">
                                Pending · Total {recMetrics?.total ?? 0}
                            </p>
                        </CardContent>
                    </Card>
                </motion.div>

                {/* Approval Rate Card */}
                <motion.div variants={cardVariants} whileHover={{ scale: 1.02, y: -2 }} transition={{ type: "spring", stiffness: 300 }}>
                    <Card className="h-full">
                        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                            <CardTitle className="text-sm font-medium">Approval Rate</CardTitle>
                            <Activity className="h-4 w-4 text-muted-foreground" />
                        </CardHeader>
                        <CardContent>
                            <div className="text-2xl font-bold" style={{ fontVariantNumeric: 'tabular-nums' }}>
                                {approvalRate}%
                            </div>
                            <p className="text-xs text-muted-foreground">
                                Last rec {lastRecTime}
                            </p>
                        </CardContent>
                    </Card>
                </motion.div>

                {/* Quality Score Card */}
                <motion.div variants={cardVariants} whileHover={{ scale: 1.02, y: -2 }} transition={{ type: "spring", stiffness: 300 }}>
                    <Card className="h-full border-emerald-500/20 bg-emerald-500/5">
                        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                            <CardTitle className="text-sm font-medium">Quality Score</CardTitle>
                            <ShieldCheck className="h-4 w-4 text-emerald-400" />
                        </CardHeader>
                        <CardContent>
                            <div className="text-2xl font-bold" style={{ fontVariantNumeric: 'tabular-nums' }}>
                                {qualityValue}
                            </div>
                            <p className="text-xs text-muted-foreground">
                                {qualityLabel} · {qualityTime}
                            </p>
                        </CardContent>
                    </Card>
                </motion.div>
            </motion.div>

            <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-7">
                <motion.div
                    className="col-span-4"
                    initial={{ opacity: 0, x: -20 }}
                    animate={{ opacity: 1, x: 0 }}
                    transition={{ delay: 0.2, duration: 0.5 }}
                >
                    <Card className="h-full mb-4">
                        <CardHeader>
                            <CardTitle>System Logs</CardTitle>
                        </CardHeader>
                        <CardContent>
                            <div className="space-y-4 max-h-40 overflow-y-auto">
                                {logsLoading ? (
                                    <p className="text-sm text-muted-foreground">Loading...</p>
                                ) : logs && logs.length > 0 ? (
                                    logs.slice(0, 5).map((log, i) => (
                                        <div
                                            key={`${log.timestamp}-${i}`}
                                            className="flex items-center gap-4 text-sm border-b border-white/5 pb-2 last:border-0"
                                        >
                                            <div className="text-muted-foreground font-mono text-xs w-24 shrink-0">
                                                {log.timestamp ? format(new Date(log.timestamp), "HH:mm:ss") : "—"}
                                            </div>
                                            <div className="truncate">{log.message}</div>
                                        </div>
                                    ))
                                ) : (
                                    <p className="text-sm text-muted-foreground">No recent logs.</p>
                                )}
                            </div>
                        </CardContent>
                    </Card>
                    <div className="h-60">
                        <AuditLog />
                    </div>
                    <Card className="h-full mt-4">
                        <CardHeader className="space-y-2">
                            <CardTitle>Verification Timeline</CardTitle>
                            <div className="flex flex-wrap gap-2 text-[11px]">
                                <select
                                    value={runKindFilter}
                                    onChange={(e) => setRunKindFilter(e.target.value)}
                                    className="rounded-md bg-white/5 border border-white/10 px-2 py-1 text-[11px]"
                                >
                                    <option value="all">All types</option>
                                    {runKinds.map((kind) => (
                                        <option key={kind} value={kind}>
                                            {kind}
                                        </option>
                                    ))}
                                </select>
                                <select
                                    value={runStatusFilter}
                                    onChange={(e) => setRunStatusFilter(e.target.value as "all" | "ok" | "fail")}
                                    className="rounded-md bg-white/5 border border-white/10 px-2 py-1 text-[11px]"
                                >
                                    <option value="all">All status</option>
                                    <option value="ok">OK only</option>
                                    <option value="fail">Fail only</option>
                                </select>
                                <span className="text-muted-foreground px-2 py-1">
                                    {filteredRuns.length} runs
                                </span>
                            </div>
                        </CardHeader>
                        <CardContent>
                            <div className="space-y-3 max-h-44 overflow-y-auto pr-1">
                                {filteredRuns.length > 0 ? (
                                    filteredRuns.slice(0, 8).map((run) => (
                                        <div
                                            key={run.id}
                                            className="flex items-start justify-between gap-3 border-b border-white/5 pb-2 last:border-0 text-xs"
                                        >
                                            <div className="space-y-1">
                                                <div className="font-semibold text-white/90">{run.kind}</div>
                                                <div className="text-muted-foreground line-clamp-2">
                                                    {run.summary}
                                                </div>
                                                <div className="text-[10px] text-muted-foreground">
                                                    {format(new Date(run.created_at), "HH:mm:ss")}
                                                </div>
                                                {run.details && (
                                                    <button
                                                        onClick={() =>
                                                            setExpandedRunId((prev) => (prev === run.id ? null : run.id))
                                                        }
                                                        className="text-[10px] text-indigo-200 hover:text-indigo-100"
                                                    >
                                                        {expandedRunId === run.id ? "Hide details" : "View details"}
                                                    </button>
                                                )}
                                                {expandedRunId === run.id && run.details && (
                                                    <pre className="mt-2 max-h-32 overflow-auto rounded-md bg-black/40 p-2 text-[10px] text-white/80 whitespace-pre-wrap">
                                                        {formatRunDetails(run.details)}
                                                    </pre>
                                                )}
                                            </div>
                                            <div className={`shrink-0 text-[10px] px-2 py-0.5 rounded-full ${run.ok ? "bg-emerald-500/20 text-emerald-200" : "bg-rose-500/20 text-rose-200"}`}>
                                                {run.ok ? "OK" : "FAIL"}
                                            </div>
                                        </div>
                                    ))
                                ) : (
                                    <p className="text-sm text-muted-foreground">No verification runs yet.</p>
                                )}
                            </div>
                        </CardContent>
                    </Card>
                </motion.div>

                <motion.div
                    className="col-span-3"
                    initial={{ opacity: 0, x: 20 }}
                    animate={{ opacity: 1, x: 0 }}
                    transition={{ delay: 0.3, duration: 0.5 }}
                >
                    <RecommendationsCard />
                    <RoutinesCard />
                    <QualityGateCard />
                    <VerificationActionsCard />
                    <ExecControlsCard />
                    <BetaActionsCard />
                    <FeedbackCard />
                    <FeedbackCard />
                    <QuickActionsCard />
                </motion.div>
            </div>
        </div>
    );
}

function formatRunDetails(details: string): string {
    try {
        const parsed = JSON.parse(details);
        return JSON.stringify(parsed, null, 2);
    } catch {
        return details;
    }
}

function formatMetricValue(value: number): string {
    if (Number.isNaN(value)) return "—";
    if (Number.isInteger(value) || Math.abs(value) >= 1000) {
        return Math.round(value).toString();
    }
    return value.toFixed(2);
}

function RecommendationsCard() {
    const { data: recs, refetch } = useRecommendations();
    const [filter, setFilter] = useState("");
    const pendingRecs = recs?.filter(r => r.status === 'pending') ?? [];
    const laterRecs = recs?.filter(r => r.status === 'later') ?? [];
    const failedRecs = recs?.filter(r => r.status === 'failed') ?? [];
    const [feedbackOpenId, setFeedbackOpenId] = useState<number | null>(null);
    const [feedbackText, setFeedbackText] = useState<Record<number, string>>({});
    const [feedbackStatus, setFeedbackStatus] = useState<Record<number, string>>({});

    const handleApprove = async (id: number) => {
        try {
            await approveRecommendation(id);
            refetch(); // Soft refresh
        } catch (e) {
            console.error("Failed to approve", e);
        }
    };

    const handleReject = async (id: number) => {
        try {
            await rejectRecommendation(id);
            refetch(); // Remove from list
        } catch (e) {
            console.error("Failed to reject", e);
        }
    };

    const handleLater = async (id: number) => {
        try {
            await laterRecommendation(id);
            refetch();
        } catch (e) {
            console.error("Failed to defer", e);
        }
    };

    const handleRestore = async (id: number) => {
        try {
            await restoreRecommendation(id);
            refetch();
        } catch (e) {
            console.error("Failed to restore", e);
        }
    };

    const handleFeedbackSubmit = async (recId: number, goal: string, summary: string, evidence?: string[]) => {
        const text = (feedbackText[recId] || "").trim();
        if (!text) {
            setFeedbackStatus((prev) => ({ ...prev, [recId]: "Feedback is required." }));
            return;
        }
        try {
            const history = [
                `Recommendation: ${goal}`,
                summary ? `Summary: ${summary}` : "",
                evidence && evidence.length > 0 ? `Evidence: ${evidence.slice(0, 2).join(" | ")}` : "",
            ].filter(Boolean).join(" / ");
            const res = await sendFeedback(goal, text, history);
            setFeedbackStatus((prev) => ({ ...prev, [recId]: res.message || "Feedback submitted." }));
            setFeedbackText((prev) => ({ ...prev, [recId]: "" }));
        } catch {
            setFeedbackStatus((prev) => ({ ...prev, [recId]: "Failed to submit feedback." }));
        }
    };

    const filterText = filter.trim().toLowerCase();
    const applyFilter = (items: typeof pendingRecs) =>
        filterText
            ? items.filter(r =>
                r.title.toLowerCase().includes(filterText) ||
                r.summary.toLowerCase().includes(filterText)
            )
            : items;
    const filteredPending = applyFilter(pendingRecs);
    const filteredLater = applyFilter(laterRecs);
    const filteredFailed = applyFilter(failedRecs);

    if (pendingRecs.length === 0 && laterRecs.length === 0 && failedRecs.length === 0) return null;

    return (
        <Card className="h-auto mb-4 border-yellow-500/20 bg-yellow-500/5">
            <CardHeader>
                <CardTitle className="flex items-center gap-2 text-yellow-500">
                    <span className="relative flex h-3 w-3">
                        <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-yellow-400 opacity-75"></span>
                        <span className="relative inline-flex rounded-full h-3 w-3 bg-yellow-500"></span>
                    </span>
                    Proposals ({filteredPending.length})
                </CardTitle>
            </CardHeader>
            <CardContent>
                <div className="mb-3">
                    <input
                        value={filter}
                        onChange={(e) => setFilter(e.target.value)}
                        placeholder="Filter by title or summary..."
                        className="w-full rounded-md bg-white/5 border border-white/10 px-3 py-2 text-xs"
                    />
                </div>
                <div className="space-y-4">
                    {filteredPending.map(rec => (
                        <div key={rec.id} className="p-3 bg-black/40 rounded-lg border border-white/10 relative overflow-hidden group">
                            {/* Confidence Indicator */}
                            <div className="absolute top-0 right-0 p-1">
                                <span className={`text-[10px] px-1.5 py-0.5 rounded-bl-md font-mono ${rec.confidence > 0.8 ? 'bg-green-500/20 text-green-400' : 'bg-yellow-500/20 text-yellow-400'}`}>
                                    {(rec.confidence * 100).toFixed(0)}%
                                </span>
                            </div>

                            <h4 className="font-semibold text-sm mb-1 pr-8">{rec.title}</h4>
                            <p className="text-xs text-muted-foreground mb-2">{rec.summary}</p>

                            {/* [Evidence UI] */}
                            {rec.evidence && rec.evidence.length > 0 && (
                                <div className="mb-3 p-2.5 bg-indigo-500/10 rounded-md border border-indigo-500/20 text-[11px] space-y-1.5">
                                    <div className="flex items-center gap-1.5 text-indigo-400 font-bold uppercase tracking-wide">
                                        <Lightbulb className="w-3.5 h-3.5" />
                                        <span>Why this?</span>
                                    </div>
                                    <ul className="space-y-1 text-gray-300 leading-tight">
                                        {rec.evidence.slice(0, 3).map((ev, i) => (
                                            <li key={i} className="flex gap-1.5 items-start">
                                                <span className="text-indigo-500/50 block mt-0.5">•</span>
                                                <span className="opacity-90">{ev}</span>
                                            </li>
                                        ))}
                                    </ul>
                                </div>
                            )}

                            <div className="flex gap-2">
                                <button
                                    onClick={() => handleApprove(rec.id)}
                                    className="flex-1 bg-green-600 hover:bg-green-700 text-white text-xs py-1.5 rounded transition-colors font-medium"
                                >
                                    Approve
                                </button>
                                <button
                                    onClick={() => handleLater(rec.id)}
                                    className="flex-1 bg-white/10 hover:bg-white/20 text-xs py-1.5 rounded transition-colors"
                                >
                                    Later
                                </button>
                                <button
                                    onClick={() => handleReject(rec.id)}
                                    className="flex-1 bg-red-500/20 hover:bg-red-500/30 text-xs py-1.5 rounded transition-colors text-red-200"
                                >
                                    Reject
                                </button>
                            </div>

                            <button
                                onClick={() => setFeedbackOpenId(feedbackOpenId === rec.id ? null : rec.id)}
                                className="mt-2 text-[11px] text-indigo-200 hover:text-indigo-100"
                            >
                                {feedbackOpenId === rec.id ? "Hide feedback" : "Send feedback"}
                            </button>

                            {feedbackOpenId === rec.id && (
                                <div className="mt-2 space-y-2">
                                    <textarea
                                        value={feedbackText[rec.id] || ""}
                                        onChange={(e) => setFeedbackText((prev) => ({ ...prev, [rec.id]: e.target.value }))}
                                        placeholder="What should be refined?"
                                        rows={2}
                                        className="w-full rounded-md bg-white/5 border border-white/10 px-2 py-1.5 text-xs"
                                    />
                                    {feedbackStatus[rec.id] && (
                                        <div className="text-[11px] text-muted-foreground">{feedbackStatus[rec.id]}</div>
                                    )}
                                    <button
                                        onClick={() => handleFeedbackSubmit(rec.id, rec.title, rec.summary, rec.evidence)}
                                        className="w-full bg-white/10 hover:bg-white/20 text-xs py-1.5 rounded transition-colors"
                                    >
                                        Submit Feedback
                                    </button>
                                </div>
                            )}
                        </div>
                    ))}
                </div>

                {filteredLater.length > 0 && (
                    <div className="mt-5 pt-4 border-t border-white/10">
                        <div className="text-xs uppercase tracking-wide text-muted-foreground mb-2">
                            Later ({filteredLater.length})
                        </div>
                        <div className="space-y-2">
                            {filteredLater.slice(0, 3).map(rec => (
                                <div key={rec.id} className="flex items-center justify-between text-xs bg-white/5 rounded px-2 py-1.5">
                                    <span className="truncate">{rec.title}</span>
                                    <button
                                        onClick={() => handleRestore(rec.id)}
                                        className="text-indigo-300 hover:text-indigo-200"
                                    >
                                        Show again
                                    </button>
                                </div>
                            ))}
                        </div>
                    </div>
                )}

                {filteredFailed.length > 0 && (
                    <div className="mt-5 pt-4 border-t border-white/10">
                        <div className="text-xs uppercase tracking-wide text-muted-foreground mb-2">
                            Failed ({filteredFailed.length})
                        </div>
                        <div className="space-y-2">
                            {filteredFailed.slice(0, 2).map(rec => (
                                <div key={rec.id} className="text-xs bg-red-500/10 border border-red-500/20 rounded px-2 py-1.5">
                                    <div className="font-medium truncate">{rec.title}</div>
                                    <div className="text-red-200/80 truncate">
                                        {rec.last_error ?? "Unknown error"}
                                    </div>
                                    <div className="mt-2 flex gap-2">
                                        <button
                                            onClick={() => handleApprove(rec.id)}
                                            className="text-[11px] px-2 py-1 rounded bg-red-500/20 hover:bg-red-500/30 text-red-200"
                                        >
                                            Retry
                                        </button>
                                    </div>
                                </div>
                            ))}
                        </div>
                    </div>
                )}
            </CardContent>
        </Card>
    );
}

function ExecControlsCard() {
    const { data: allowlist, refetch: refetchAllowlist } = useExecAllowlist(100);
    const [execStatusFilter, setExecStatusFilter] = useState("all");
    const { data: execResults, refetch: refetchExecResults } = useExecResults(60, execStatusFilter === "all" ? undefined : execStatusFilter);
    const [pattern, setPattern] = useState("");
    const [cwd, setCwd] = useState("");
    const [status, setStatus] = useState<string | null>(null);
    const [guardStatus, setGuardStatus] = useState<string | null>(null);
    const [guardAge, setGuardAge] = useState("300");
    const [expandedExecId, setExpandedExecId] = useState<string | null>(null);

    const handleAddAllowlist = async () => {
        if (!pattern.trim()) {
            setStatus("Pattern is required.");
            return;
        }
        try {
            await addExecAllowlist(pattern.trim(), cwd.trim() || undefined);
            setPattern("");
            setCwd("");
            setStatus("Allowlist entry added.");
            refetchAllowlist();
        } catch {
            setStatus("Failed to add allowlist entry.");
        }
    };

    const handleRemoveAllowlist = async (id: number) => {
        try {
            await removeExecAllowlist(id);
            setStatus("Allowlist entry removed.");
            refetchAllowlist();
        } catch {
            setStatus("Failed to remove allowlist entry.");
        }
    };

    const { data: approvals, refetch: refetchApprovals } = useExecApprovals("pending");

    const handleApproveExec = async (id: string) => {
        try {
            await approveExecApproval(id, "Dashboard User");
            setStatus("Approved execution.");
            refetchApprovals();
        } catch {
            setStatus("Failed to approve.");
        }
    };

    const handleRejectExec = async (id: string) => {
        try {
            await rejectExecApproval(id, "Dashboard User");
            setStatus("Rejected execution.");
            refetchApprovals();
        } catch {
            setStatus("Failed to reject.");
        }
    };

    const handleGuard = async () => {
        const maxAge = Number.isFinite(Number(guardAge)) ? Number(guardAge) : undefined;
        try {
            const res = await runExecResultsGuard(maxAge, 200);
            setGuardStatus(`Guarded ${res.scanned}, timed out ${res.timed_out}`);
            refetchExecResults();
        } catch {
            setGuardStatus("Guard failed.");
        }
    };

    return (
        <Card className="h-auto mb-4">
            <CardHeader>
                <CardTitle>Exec Controls</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
                {approvals && approvals.length > 0 && (
                    <div className="space-y-2 border-b border-white/10 pb-4">
                        <div className="flex items-center justify-between text-xs">
                            <span className="font-semibold text-amber-300">⚠️ Pending Approvals</span>
                            <span className="text-muted-foreground">{approvals.length} request(s)</span>
                        </div>
                        <div className="space-y-2 max-h-32 overflow-y-auto">
                            {approvals.map((approval) => (
                                <div key={approval.id} className="bg-amber-500/10 border border-amber-500/20 p-2 rounded">
                                    <div className="text-[11px] font-mono mb-1 break-all">{approval.command}</div>
                                    <div className="flex gap-2">
                                        <button
                                            onClick={() => handleApproveExec(approval.id)}
                                            className="flex-1 bg-emerald-500/20 hover:bg-emerald-500/30 text-emerald-200 text-[10px] py-1 rounded transition-colors"
                                        >
                                            Approve
                                        </button>
                                        <button
                                            onClick={() => handleRejectExec(approval.id)}
                                            className="flex-1 bg-rose-500/20 hover:bg-rose-500/30 text-rose-200 text-[10px] py-1 rounded transition-colors"
                                        >
                                            Reject
                                        </button>
                                    </div>
                                </div>
                            ))}
                        </div>
                    </div>
                )}
                <div className="space-y-2">
                    <div className="text-xs text-muted-foreground">Allowlist</div>
                    <div className="flex gap-2">
                        <input
                            value={pattern}
                            onChange={(e) => setPattern(e.target.value)}
                            placeholder="Pattern (e.g. git status)"
                            className="flex-1 rounded-md bg-white/5 border border-white/10 px-2 py-1 text-xs"
                        />
                        <input
                            value={cwd}
                            onChange={(e) => setCwd(e.target.value)}
                            placeholder="CWD (optional)"
                            className="flex-1 rounded-md bg-white/5 border border-white/10 px-2 py-1 text-xs"
                        />
                    </div>
                    <button
                        onClick={handleAddAllowlist}
                        className="w-full text-xs py-1.5 rounded bg-white/10 hover:bg-white/20 transition-colors"
                    >
                        Add allowlist entry
                    </button>
                    {status && <div className="text-[11px] text-muted-foreground">{status}</div>}
                    <div className="space-y-1 max-h-24 overflow-y-auto text-[11px]">
                        {allowlist && allowlist.length > 0 ? (
                            allowlist.slice(0, 10).map((item) => (
                                <div key={item.id} className="flex items-center justify-between gap-2 bg-white/5 px-2 py-1 rounded">
                                    <div className="truncate">
                                        {item.pattern}
                                        {item.cwd ? ` · ${item.cwd}` : ""}
                                    </div>
                                    <button
                                        onClick={() => handleRemoveAllowlist(item.id)}
                                        className="text-red-200 hover:text-red-100"
                                    >
                                        Remove
                                    </button>
                                </div>
                            ))
                        ) : (
                            <div className="text-muted-foreground">No allowlist entries.</div>
                        )}
                    </div>
                </div>

                <div className="space-y-2">
                    <div className="text-xs text-muted-foreground">Exec Results</div>
                    <div className="flex gap-2">
                        <select
                            value={execStatusFilter}
                            onChange={(e) => setExecStatusFilter(e.target.value)}
                            className="flex-1 rounded-md bg-white/5 border border-white/10 px-2 py-1 text-xs"
                        >
                            <option value="all">All status</option>
                            <option value="pending">pending</option>
                            <option value="success">success</option>
                            <option value="error">error</option>
                            <option value="timeout">timeout</option>
                        </select>
                        <input
                            value={guardAge}
                            onChange={(e) => setGuardAge(e.target.value)}
                            placeholder="Max age secs (300)"
                            className="flex-1 rounded-md bg-white/5 border border-white/10 px-2 py-1 text-xs"
                        />
                        <button
                            onClick={handleGuard}
                            className="text-xs px-3 py-1 rounded bg-white/10 hover:bg-white/20 transition-colors"
                        >
                            Run guard
                        </button>
                    </div>
                    {guardStatus && <div className="text-[11px] text-muted-foreground">{guardStatus}</div>}
                    <div className="space-y-1 max-h-24 overflow-y-auto text-[11px]">
                        {execResults && execResults.length > 0 ? (
                            execResults.slice(0, 8).map((item) => (
                                <div key={item.id} className="bg-white/5 px-2 py-1 rounded">
                                    <div className="flex items-center justify-between gap-2">
                                        <div className="truncate">
                                            {item.command} · {item.status}
                                        </div>
                                        <span className="text-[10px] text-muted-foreground">
                                            {item.updated_at ? format(new Date(item.updated_at), "HH:mm:ss") : "—"}
                                        </span>
                                    </div>
                                    {(item.output || item.error) && (
                                        <button
                                            onClick={() => setExpandedExecId((prev) => (prev === item.id ? null : item.id))}
                                            className="text-[10px] text-indigo-200 hover:text-indigo-100 mt-1"
                                        >
                                            {expandedExecId === item.id ? "Hide details" : "View details"}
                                        </button>
                                    )}
                                    {expandedExecId === item.id && (
                                        <pre className="mt-1 max-h-24 overflow-auto rounded-md bg-black/40 p-2 text-[10px] text-white/80 whitespace-pre-wrap">
                                            {item.output || item.error}
                                        </pre>
                                    )}
                                </div>
                            ))
                        ) : (
                            <div className="text-muted-foreground">No exec results.</div>
                        )}
                    </div>
                </div>
            </CardContent>
        </Card>
    );
}

function QuickActionsCard() {
    const [showRoutine, setShowRoutine] = useState(false);
    const [routineName, setRoutineName] = useState("");
    const [routineCron, setRoutineCron] = useState("");
    const [routinePrompt, setRoutinePrompt] = useState("");
    const [routineLoading, setRoutineLoading] = useState(false);

    const [analyzing, setAnalyzing] = useState(false);
    const [patterns, setPatterns] = useState<string[] | null>(null);

    const handleCreateRoutine = async () => {
        if (!routineName || !routineCron || !routinePrompt) return;
        setRoutineLoading(true);
        try {
            await createRoutine(routineName, routineCron, routinePrompt);
            setShowRoutine(false);
            setRoutineName("");
            setRoutineCron("");
            setRoutinePrompt("");
            // Ideally refetch routines here, but we can rely on auto-refetch
        } catch {
            alert("Failed to create routine");
        } finally {
            setRoutineLoading(false);
        }
    };

    const handleAnalyze = async () => {
        setAnalyzing(true);
        try {
            const res = await analyzePatterns();
            setPatterns(res);
        } catch {
            setPatterns([]);
        } finally {
            setAnalyzing(false);
        }
    };

    return (
        <Card className="h-auto">
            <CardHeader>
                <CardTitle>Quick Actions</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2">
                <Dialog open={showRoutine} onOpenChange={setShowRoutine}>
                    <DialogTrigger asChild>
                        <button className="w-full text-left px-4 py-2 rounded-lg hover:bg-white/5 transition-colors text-sm">
                            ➕ Create New Routine
                        </button>
                    </DialogTrigger>
                    <DialogContent className="bg-[#1a1a1a] border-white/10 text-white">
                        <DialogHeader>
                            <DialogTitle>Create New Routine</DialogTitle>
                        </DialogHeader>
                        <div className="space-y-3 py-4">
                            <input
                                placeholder="Routine Name"
                                value={routineName}
                                onChange={e => setRoutineName(e.target.value)}
                                className="w-full bg-black/20 border border-white/10 rounded px-3 py-2 text-sm"
                            />
                            <input
                                placeholder="Cron Expression (e.g. 0 9 * * *)"
                                value={routineCron}
                                onChange={e => setRoutineCron(e.target.value)}
                                className="w-full bg-black/20 border border-white/10 rounded px-3 py-2 text-sm font-mono"
                            />
                            <textarea
                                placeholder="What should the agent do?"
                                value={routinePrompt}
                                onChange={e => setRoutinePrompt(e.target.value)}
                                rows={3}
                                className="w-full bg-black/20 border border-white/10 rounded px-3 py-2 text-sm"
                            />
                            <button
                                onClick={handleCreateRoutine}
                                disabled={routineLoading}
                                className="w-full bg-indigo-500 hover:bg-indigo-600 text-white py-2 rounded"
                            >
                                {routineLoading ? "Creating..." : "Create Routine"}
                            </button>
                        </div>
                    </DialogContent>
                </Dialog>

                <button
                    onClick={handleAnalyze}
                    disabled={analyzing}
                    className="w-full text-left px-4 py-2 rounded-lg hover:bg-white/5 transition-colors text-sm flex justify-between items-center"
                >
                    <span>🔍 Analyze Patterns</span>
                    {analyzing && <span className="text-xs text-muted-foreground">...</span>}
                </button>

                {patterns && (
                    <div className="bg-black/20 p-2 rounded text-xs space-y-1">
                        {patterns.length > 0 ? (
                            patterns.map((p, i) => <div key={i}>• {p}</div>)
                        ) : (
                            <div className="text-muted-foreground">No patterns found.</div>
                        )}
                        <button onClick={() => setPatterns(null)} className="text-[10px] text-white/50 w-full text-center mt-1">Close</button>
                    </div>
                )}

                <button
                    onClick={() => alert("Settings are currently managed via config.toml")}
                    className="w-full text-left px-4 py-2 rounded-lg hover:bg-white/5 transition-colors text-sm"
                >
                    ⚙️ Open Settings
                </button>
            </CardContent>
        </Card>
    );
}

function QualityGateCard() {
    const { data: qualityScore, refetch: refetchQuality, isFetching: qualityFetching } = useQualityScore();
    const { data: consistency, refetch: refetchConsistency, isFetching: consistencyFetching } = useConsistencyCheck();
    const { data: semantic, refetch: refetchSemantic, isFetching: semanticFetching } = useSemanticVerification();
    const [gateOverrides, setGateOverrides] = useState<ReleaseGateOverrides | undefined>(undefined);
    const [perfInput, setPerfInput] = useState("");
    const [qualityInput, setQualityInput] = useState("");
    const { data: releaseGate, refetch: refetchReleaseGate, isFetching: releaseFetching } = useReleaseGate(gateOverrides);
    const [expanded, setExpanded] = useState(false);
    const [issueSearch, setIssueSearch] = useState("");
    const [issueTypeFilter, setIssueTypeFilter] = useState("all");
    const [issueSeverityFilter, setIssueSeverityFilter] = useState("all");

    const loading = qualityFetching || consistencyFetching || semanticFetching || releaseFetching;
    const handleRefresh = async () => {
        await Promise.all([
            refetchQuality(),
            refetchConsistency(),
            refetchSemantic(),
            refetchReleaseGate(),
        ]);
    };

    const qualityValue = qualityScore?.score?.overall?.toFixed(1) ?? "—";
    const qualityLabel = qualityScore?.score?.recommendation ?? "pending";
    const qualityTime = qualityScore?.created_at
        ? format(new Date(qualityScore.created_at), "HH:mm")
        : "—";

    const gateOk = releaseGate?.ok ?? false;
    const gateWarnings = releaseGate?.warnings?.length ?? 0;
    const gateRegressions = releaseGate?.regressions?.length ?? 0;
    const gateStatus = releaseGate
        ? gateOk
            ? gateWarnings > 0
                ? `PASS · ${gateWarnings} warn`
                : "PASS"
            : `FAIL · ${gateRegressions}`
        : "—";
    const gateTime = releaseGate?.current?.created_at
        ? format(new Date(releaseGate.current.created_at), "HH:mm")
        : "—";

    const consistencyCount = consistency?.issues?.length ?? 0;
    const semanticCount = semantic?.issues?.length ?? 0;
    const hasAnyIssues = gateRegressions > 0 || gateWarnings > 0 || consistencyCount > 0 || semanticCount > 0;

    const handleApplyGateOverrides = () => {
        const perf = Number.isFinite(Number(perfInput)) ? Number(perfInput) : undefined;
        const quality = Number.isFinite(Number(qualityInput)) ? Number(qualityInput) : undefined;
        if (perf === undefined && quality === undefined) {
            setGateOverrides(undefined);
        } else {
            setGateOverrides({
                perf_regression_pct: perf,
                quality_drop: quality,
            });
        }
        refetchReleaseGate();
    };

    const handleResetGateOverrides = () => {
        setPerfInput("");
        setQualityInput("");
        setGateOverrides(undefined);
        refetchReleaseGate();
    };

    const gateTone = !releaseGate
        ? "border-white/10 bg-white/5"
        : gateOk
            ? gateWarnings > 0
                ? "border-amber-500/20 bg-amber-500/5"
                : "border-emerald-500/20 bg-emerald-500/5"
            : "border-rose-500/20 bg-rose-500/5";

    const hasOverrides = gateOverrides?.perf_regression_pct !== undefined || gateOverrides?.quality_drop !== undefined;

    const issueItems = [
        ...(releaseGate?.regressions ?? []).map((item) => ({
            kind: "release_regression",
            severity: "high",
            tone: "rose",
            text: item,
        })),
        ...(releaseGate?.warnings ?? []).map((item) => ({
            kind: "release_warning",
            severity: "medium",
            tone: "amber",
            text: item,
        })),
        ...(consistency?.issues ?? []).map((issue) => ({
            kind: "consistency",
            severity: "medium",
            tone: "rose",
            text: `${issue.path} · ${issue.source}`,
        })),
        ...(semantic?.issues ?? []).map((issue) => ({
            kind: "semantic",
            severity: issue.severity,
            tone: "amber",
            text: `${issue.file} (${issue.severity}) · ${issue.reason}`,
        })),
    ];

    const filteredIssueItems = issueItems.filter((item) => {
        if (issueTypeFilter !== "all" && item.kind !== issueTypeFilter) return false;
        if (issueSeverityFilter !== "all" && item.severity !== issueSeverityFilter) return false;
        if (issueSearch) {
            return item.text.toLowerCase().includes(issueSearch.toLowerCase());
        }
        return true;
    });

    const issuesByKind = (kind: string) => filteredIssueItems.filter((item) => item.kind === kind).map((item) => item.text);

    return (
        <Card className={`h-auto mb-4 ${gateTone}`}>
            <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">Quality & Gate</CardTitle>
                <div className="flex gap-2">
                    <button
                        onClick={async () => {
                            await calculateQualityScore();
                            handleRefresh();
                        }}
                        className="text-[10px] bg-white/10 hover:bg-white/20 px-2 py-1 rounded transition-colors"
                    >
                        🔄 Recalculate
                    </button>
                    <ShieldCheck className="h-4 w-4 text-emerald-400" />
                </div>
            </CardHeader>
            <CardContent className="space-y-3">
                <div className="flex items-center justify-between text-xs">
                    <span className="text-muted-foreground">Quality score</span>
                    <span className="text-emerald-300">
                        {qualityValue} · {qualityLabel}
                    </span>
                </div>
                <div className="flex items-center justify-between text-xs">
                    <span className="text-muted-foreground">Release gate</span>
                    <span className={gateOk ? "text-emerald-400" : "text-rose-400"}>
                        {gateStatus}
                    </span>
                </div>
                <div className="flex items-center justify-between text-[11px] text-muted-foreground">
                    <span>Last gate run</span>
                    <span>{gateTime}</span>
                </div>
                <div className="flex items-center justify-between text-xs">
                    <span className="text-muted-foreground">API consistency</span>
                    <span className={consistency?.ok ? "text-emerald-400" : "text-rose-400"}>
                        {consistency?.ok ? "OK" : "Issue"} · {consistencyCount}
                    </span>
                </div>
                <div className="flex items-center justify-between text-xs">
                    <span className="text-muted-foreground">Static/Semantic</span>
                    <span className={semantic?.ok ? "text-emerald-400" : "text-rose-400"}>
                        {semantic?.ok ? "OK" : "Issue"} · {semanticCount}
                    </span>
                </div>
                <div className="text-[11px] text-muted-foreground">
                    Last score {qualityTime}
                </div>
                <div className="rounded-md border border-white/10 bg-black/20 p-2 space-y-2">
                    <div className="text-[11px] text-muted-foreground">Gate thresholds (optional)</div>
                    <div className="grid grid-cols-2 gap-2">
                        <input
                            value={perfInput}
                            onChange={(e) => setPerfInput(e.target.value)}
                            placeholder="Perf regression (0.1)"
                            className="w-full rounded-md bg-white/5 border border-white/10 px-2 py-1 text-[11px]"
                        />
                        <input
                            value={qualityInput}
                            onChange={(e) => setQualityInput(e.target.value)}
                            placeholder="Quality drop (0.3)"
                            className="w-full rounded-md bg-white/5 border border-white/10 px-2 py-1 text-[11px]"
                        />
                    </div>
                    <div className="flex gap-2">
                        <button
                            onClick={handleApplyGateOverrides}
                            className="flex-1 text-[11px] py-1 rounded bg-white/10 hover:bg-white/20 transition-colors"
                        >
                            Apply
                        </button>
                        <button
                            onClick={handleResetGateOverrides}
                            className="flex-1 text-[11px] py-1 rounded bg-white/5 hover:bg-white/10 transition-colors"
                        >
                            Reset
                        </button>
                    </div>
                    {hasOverrides && (
                        <div className="text-[10px] text-amber-200">
                            Overrides active
                        </div>
                    )}
                </div>
                {hasAnyIssues && (
                    <button
                        onClick={() => setExpanded((prev) => !prev)}
                        className="text-[11px] text-indigo-200 hover:text-indigo-100"
                    >
                        {expanded ? "Hide details" : "Show details"}
                    </button>
                )}
                {hasAnyIssues && (
                    <Dialog>
                        <DialogTrigger asChild>
                            <button className="text-[11px] text-indigo-200 hover:text-indigo-100">
                                Open issue list
                            </button>
                        </DialogTrigger>
                        <DialogContent className="max-w-2xl">
                            <DialogHeader>
                                <DialogTitle>Verification Issues</DialogTitle>
                            </DialogHeader>
                            <div className="space-y-3">
                                <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
                                    <input
                                        value={issueSearch}
                                        onChange={(e) => setIssueSearch(e.target.value)}
                                        placeholder="Filter issues..."
                                        className="w-full rounded-md bg-white/5 border border-white/10 px-3 py-2 text-sm"
                                    />
                                    <div className="flex gap-2">
                                        <select
                                            value={issueTypeFilter}
                                            onChange={(e) => setIssueTypeFilter(e.target.value)}
                                            className="flex-1 rounded-md bg-white/5 border border-white/10 px-2 py-2 text-sm"
                                        >
                                            <option value="all">All types</option>
                                            <option value="release_regression">Release regressions</option>
                                            <option value="release_warning">Release warnings</option>
                                            <option value="consistency">API consistency</option>
                                            <option value="semantic">Static/Semantic</option>
                                        </select>
                                        <select
                                            value={issueSeverityFilter}
                                            onChange={(e) => setIssueSeverityFilter(e.target.value)}
                                            className="flex-1 rounded-md bg-white/5 border border-white/10 px-2 py-2 text-sm"
                                        >
                                            <option value="all">All severity</option>
                                            <option value="high">High</option>
                                            <option value="medium">Medium</option>
                                            <option value="low">Low</option>
                                        </select>
                                    </div>
                                </div>
                                <div className="space-y-3 max-h-[60vh] overflow-y-auto pr-1">
                                    {issuesByKind("release_regression").length > 0 && (
                                        <IssueSection
                                            title="Release Gate · Regressions"
                                            tone="rose"
                                            items={issuesByKind("release_regression")}
                                            filter=""
                                        />
                                    )}
                                    {issuesByKind("release_warning").length > 0 && (
                                        <IssueSection
                                            title="Release Gate · Warnings"
                                            tone="amber"
                                            items={issuesByKind("release_warning")}
                                            filter=""
                                        />
                                    )}
                                    {issuesByKind("consistency").length > 0 && (
                                        <IssueSection
                                            title="API Consistency"
                                            tone="rose"
                                            items={issuesByKind("consistency")}
                                            filter=""
                                        />
                                    )}
                                    {issuesByKind("semantic").length > 0 && (
                                        <IssueSection
                                            title="Static/Semantic"
                                            tone="amber"
                                            items={issuesByKind("semantic")}
                                            filter=""
                                        />
                                    )}
                                    {filteredIssueItems.length === 0 && (
                                        <div className="text-sm text-muted-foreground">No issues detected.</div>
                                    )}
                                </div>
                            </div>
                        </DialogContent>
                    </Dialog>
                )}
                {expanded && (
                    <div className="space-y-2 text-[11px]">
                        {gateRegressions > 0 && (
                            <div className="rounded-md border border-rose-500/20 bg-rose-500/10 p-2">
                                <div className="font-semibold text-rose-200">Release gate</div>
                                <ul className="mt-1 space-y-1 text-rose-100/80">
                                    {releaseGate?.regressions.slice(0, 3).map((item, idx) => (
                                        <li key={`reg-${idx}`} className="truncate">• {item}</li>
                                    ))}
                                </ul>
                            </div>
                        )}
                        {gateWarnings > 0 && (
                            <div className="rounded-md border border-amber-500/20 bg-amber-500/10 p-2">
                                <div className="font-semibold text-amber-200">Gate warnings</div>
                                <ul className="mt-1 space-y-1 text-amber-100/80">
                                    {releaseGate?.warnings.slice(0, 3).map((item, idx) => (
                                        <li key={`warn-${idx}`} className="truncate">• {item}</li>
                                    ))}
                                </ul>
                            </div>
                        )}
                        {consistencyCount > 0 && (
                            <div className="rounded-md border border-rose-500/20 bg-rose-500/10 p-2">
                                <div className="font-semibold text-rose-200">API Consistency</div>
                                <ul className="mt-1 space-y-1 text-rose-100/80">
                                    {consistency?.issues.slice(0, 3).map((issue, idx) => (
                                        <li key={`${issue.path}-${idx}`} className="truncate">
                                            • {issue.path}
                                        </li>
                                    ))}
                                </ul>
                            </div>
                        )}
                        {semanticCount > 0 && (
                            <div className="rounded-md border border-amber-500/20 bg-amber-500/10 p-2">
                                <div className="font-semibold text-amber-200">Static/Semantic</div>
                                <ul className="mt-1 space-y-1 text-amber-100/80">
                                    {semantic?.issues.slice(0, 3).map((issue, idx) => (
                                        <li key={`${issue.file}-${idx}`} className="truncate">
                                            • {issue.file} ({issue.severity})
                                        </li>
                                    ))}
                                </ul>
                            </div>
                        )}
                    </div>
                )}
                <button
                    onClick={handleRefresh}
                    className="w-full text-[11px] py-1.5 rounded bg-white/10 hover:bg-white/20 transition-colors"
                    disabled={loading}
                >
                    {loading ? "Checking..." : "Run checks"}
                </button>
            </CardContent>
        </Card>
    );
}

function VerificationActionsCard() {
    const { data: releaseGate, refetch: refetchReleaseGate } = useReleaseGate();
    const [runBackend, setRunBackend] = useState(true);
    const [runFrontend, setRunFrontend] = useState(true);
    const [runBuildChecks, setRunBuildChecks] = useState(false);
    const [runE2e, setRunE2e] = useState(false);
    const [runtimeResult, setRuntimeResult] = useState<RuntimeVerifyResult | null>(null);
    const [runtimeStatus, setRuntimeStatus] = useState<string | null>(null);
    const [runtimeLoading, setRuntimeLoading] = useState(false);
    const [performanceResult, setPerformanceResult] = useState<PerformanceVerification | null>(null);
    const [performanceStatus, setPerformanceStatus] = useState<string | null>(null);
    const [performanceLoading, setPerformanceLoading] = useState(false);
    const [visualPrompts, setVisualPrompts] = useState("");
    const [visualResult, setVisualResult] = useState<VisualVerifyResult | null>(null);
    const [visualStatus, setVisualStatus] = useState<string | null>(null);
    const [visualLoading, setVisualLoading] = useState(false);
    const [baselineStatus, setBaselineStatus] = useState<string | null>(null);
    const [baselineLoading, setBaselineLoading] = useState(false);

    const baselineTime = releaseGate?.baseline?.created_at
        ? format(new Date(releaseGate.baseline.created_at), "MMM d HH:mm")
        : "—";

    const handleRunRuntime = async () => {
        setRuntimeLoading(true);
        setRuntimeStatus(null);
        try {
            const result = await runRuntimeVerification({
                run_backend: runBackend,
                run_frontend: runFrontend,
                run_build_checks: runBuildChecks,
                run_e2e: runE2e,
            });
            setRuntimeResult(result);
            const issueCount = result.issues?.length ?? 0;
            setRuntimeStatus(issueCount === 0 ? "Runtime verification OK." : `Runtime issues: ${issueCount}`);
        } catch {
            setRuntimeStatus("Runtime verification failed.");
        } finally {
            setRuntimeLoading(false);
        }
    };

    const handleRunPerformance = async () => {
        setPerformanceLoading(true);
        setPerformanceStatus(null);
        try {
            const result = await runPerformanceVerification();
            setPerformanceResult(result);
            setPerformanceStatus(result.ok ? "Performance baseline OK." : "Performance issues detected.");
        } catch {
            setPerformanceStatus("Performance verification failed.");
        } finally {
            setPerformanceLoading(false);
        }
    };

    const handleRunVisual = async () => {
        const prompts = visualPrompts
            .split("\n")
            .map((p) => p.trim())
            .filter(Boolean);
        if (prompts.length === 0) {
            setVisualStatus("Add at least one prompt.");
            return;
        }
        setVisualLoading(true);
        setVisualStatus(null);
        try {
            const result = await runVisualVerification(prompts);
            setVisualResult(result);
            const failed = result.verdicts.filter((v) => !v.ok).length;
            setVisualStatus(failed === 0 ? "Visual checks passed." : `Visual issues: ${failed}`);
        } catch {
            setVisualStatus("Visual verification failed.");
        } finally {
            setVisualLoading(false);
        }
    };

    const handleSetBaseline = async () => {
        setBaselineLoading(true);
        setBaselineStatus(null);
        try {
            const baseline = await setReleaseBaseline();
            const created = baseline.created_at
                ? format(new Date(baseline.created_at), "MMM d HH:mm")
                : "Saved";
            setBaselineStatus(`Baseline saved (${created}).`);
            refetchReleaseGate();
        } catch {
            setBaselineStatus("Failed to set baseline.");
        } finally {
            setBaselineLoading(false);
        }
    };

    const runtimeIssues = runtimeResult?.issues ?? [];
    const runtimeBackendState = runtimeResult
        ? runtimeResult.backend_started
            ? runtimeResult.backend_health
                ? "OK"
                : "Unhealthy"
            : "Not started"
        : "—";
    const runtimeFrontendState = runtimeResult
        ? runtimeResult.frontend_started
            ? runtimeResult.frontend_health
                ? "OK"
                : "Unhealthy"
            : "Not started"
        : "—";
    const backendBuildState = runtimeResult?.backend_build_ok;
    const frontendBuildState = runtimeResult?.frontend_build_ok;
    const e2eState = runtimeResult?.e2e_passed;
    const buildLabel = (value?: boolean | null) => (value === undefined || value === null ? "—" : value ? "OK" : "Fail");

    return (
        <Card className="h-auto mb-4 border-white/10 bg-white/5">
            <CardHeader>
                <CardTitle>Verification Actions</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
                <div className="space-y-2">
                    <div className="text-xs text-muted-foreground">Runtime verification</div>
                    <div className="grid grid-cols-2 gap-2 text-[11px]">
                        <label className="flex items-center gap-2">
                            <input
                                type="checkbox"
                                checked={runBackend}
                                onChange={(e) => setRunBackend(e.target.checked)}
                                className="h-3 w-3"
                            />
                            Backend
                        </label>
                        <label className="flex items-center gap-2">
                            <input
                                type="checkbox"
                                checked={runFrontend}
                                onChange={(e) => setRunFrontend(e.target.checked)}
                                className="h-3 w-3"
                            />
                            Frontend
                        </label>
                        <label className="flex items-center gap-2">
                            <input
                                type="checkbox"
                                checked={runBuildChecks}
                                onChange={(e) => setRunBuildChecks(e.target.checked)}
                                className="h-3 w-3"
                            />
                            Build checks
                        </label>
                        <label className="flex items-center gap-2">
                            <input
                                type="checkbox"
                                checked={runE2e}
                                onChange={(e) => setRunE2e(e.target.checked)}
                                className="h-3 w-3"
                            />
                            E2E
                        </label>
                    </div>
                    <button
                        onClick={handleRunRuntime}
                        disabled={runtimeLoading}
                        className="w-full text-[11px] py-1.5 rounded bg-white/10 hover:bg-white/20 transition-colors disabled:opacity-50"
                    >
                        {runtimeLoading ? "Running..." : "Run runtime verification"}
                    </button>
                    {runtimeStatus && <div className="text-[11px] text-muted-foreground">{runtimeStatus}</div>}
                    {runtimeResult && (
                        <div className="rounded-md border border-white/10 bg-black/20 p-2 text-[11px] space-y-1">
                            <div className="flex items-center justify-between">
                                <span>Backend</span>
                                <span className={runtimeBackendState === "OK" ? "text-emerald-300" : "text-rose-300"}>
                                    {runtimeBackendState}
                                </span>
                            </div>
                            <div className="flex items-center justify-between">
                                <span>Frontend</span>
                                <span className={runtimeFrontendState === "OK" ? "text-emerald-300" : "text-rose-300"}>
                                    {runtimeFrontendState}
                                </span>
                            </div>
                            <div className="flex items-center justify-between">
                                <span>Build</span>
                                <span className={backendBuildState === false || frontendBuildState === false ? "text-rose-300" : "text-muted-foreground"}>
                                    Backend {buildLabel(backendBuildState)} · Frontend {buildLabel(frontendBuildState)}
                                </span>
                            </div>
                            {runE2e && (
                                <div className="flex items-center justify-between">
                                    <span>E2E</span>
                                    <span className={e2eState ? "text-emerald-300" : "text-rose-300"}>
                                        {e2eState === undefined ? "—" : e2eState ? "OK" : "Fail"}
                                    </span>
                                </div>
                            )}
                            {runtimeIssues.length > 0 && (
                                <div className="mt-1 text-rose-200">
                                    {runtimeIssues.slice(0, 3).map((issue, idx) => (
                                        <div key={`${issue}-${idx}`} className="truncate">
                                            • {issue}
                                        </div>
                                    ))}
                                </div>
                            )}
                        </div>
                    )}
                </div>

                <div className="space-y-2">
                    <div className="text-xs text-muted-foreground">Performance baseline</div>
                    <button
                        onClick={handleRunPerformance}
                        disabled={performanceLoading}
                        className="w-full text-[11px] py-1.5 rounded bg-white/10 hover:bg-white/20 transition-colors disabled:opacity-50"
                    >
                        {performanceLoading ? "Running..." : "Run performance check"}
                    </button>
                    {performanceStatus && <div className="text-[11px] text-muted-foreground">{performanceStatus}</div>}
                    {performanceResult && (
                        <div className="rounded-md border border-white/10 bg-black/20 p-2 text-[11px] space-y-1">
                            {performanceResult.metrics.map((metric) => (
                                <div key={metric.name} className="flex items-center justify-between">
                                    <span>{metric.name}</span>
                                    <span className={metric.ok ? "text-emerald-300" : "text-rose-300"}>
                                        {formatMetricValue(metric.value)} / {formatMetricValue(metric.threshold)}
                                    </span>
                                </div>
                            ))}
                        </div>
                    )}
                </div>

                <div className="space-y-2">
                    <div className="text-xs text-muted-foreground">Visual verification</div>
                    <textarea
                        value={visualPrompts}
                        onChange={(e) => setVisualPrompts(e.target.value)}
                        placeholder="One prompt per line (e.g. Error banner is hidden)"
                        rows={2}
                        className="w-full rounded-md bg-white/5 border border-white/10 px-2 py-1.5 text-[11px]"
                    />
                    <button
                        onClick={handleRunVisual}
                        disabled={visualLoading}
                        className="w-full text-[11px] py-1.5 rounded bg-white/10 hover:bg-white/20 transition-colors disabled:opacity-50"
                    >
                        {visualLoading ? "Running..." : "Run visual check"}
                    </button>
                    {visualStatus && <div className="text-[11px] text-muted-foreground">{visualStatus}</div>}
                    {visualResult && (
                        <div className="rounded-md border border-white/10 bg-black/20 p-2 text-[11px] space-y-1">
                            {visualResult.verdicts.length === 0 ? (
                                <div className="text-muted-foreground">No prompts checked.</div>
                            ) : (
                                visualResult.verdicts.slice(0, 4).map((verdict) => (
                                    <div key={verdict.prompt} className="flex items-center justify-between gap-2">
                                        <span className="truncate">{verdict.prompt}</span>
                                        <span className={verdict.ok ? "text-emerald-300" : "text-rose-300"}>
                                            {verdict.ok ? "OK" : "Fail"}
                                        </span>
                                    </div>
                                ))
                            )}
                        </div>
                    )}
                </div>

                <div className="space-y-2">
                    <div className="flex items-center justify-between text-xs text-muted-foreground">
                        <span>Release baseline</span>
                        <span>{baselineTime}</span>
                    </div>
                    <button
                        onClick={handleSetBaseline}
                        disabled={baselineLoading}
                        className="w-full text-[11px] py-1.5 rounded bg-white/10 hover:bg-white/20 transition-colors disabled:opacity-50"
                    >
                        {baselineLoading ? "Saving..." : "Set release baseline"}
                    </button>
                    {baselineStatus && <div className="text-[11px] text-muted-foreground">{baselineStatus}</div>}
                </div>
                <VerificationRunHistory />
            </CardContent>
        </Card>
    );
}

function IssueSection({
    title,
    tone,
    items,
    filter,
}: {
    title: string;
    tone: "rose" | "amber";
    items: string[];
    filter: string;
}) {
    const filtered = filter
        ? items.filter((item) => item.toLowerCase().includes(filter.toLowerCase()))
        : items;
    if (filtered.length === 0) return null;

    const classes =
        tone === "rose"
            ? "border-rose-500/20 bg-rose-500/10 text-rose-100/80"
            : "border-amber-500/20 bg-amber-500/10 text-amber-100/80";

    return (
        <div className={`rounded-md border p-3 ${classes}`}>
            <div className="font-semibold text-white/90 mb-2">{title}</div>
            <ul className="space-y-1 text-[12px]">
                {filtered.map((item, idx) => (
                    <li key={`${title}-${idx}`} className="break-words">
                        • {item}
                    </li>
                ))}
            </ul>
        </div>
    );
}

function FeedbackCard() {
    const [goal, setGoal] = useState("");
    const [feedback, setFeedback] = useState("");
    const [summary, setSummary] = useState("");
    const [status, setStatus] = useState<string | null>(null);
    const [loading, setLoading] = useState(false);

    useEffect(() => {
        let mounted = true;
        fetchCurrentGoal()
            .then((g) => {
                if (mounted && g && !goal) {
                    setGoal(g);
                }
            })
            .catch(() => { });
        return () => {
            mounted = false;
        };
    }, [goal]);

    const handleSubmit = async () => {
        if (!goal.trim() || !feedback.trim()) {
            setStatus("Goal and feedback are required.");
            return;
        }
        setLoading(true);
        setStatus(null);
        try {
            const res = await sendFeedback(goal.trim(), feedback.trim(), summary.trim() || undefined);
            setStatus(res.message || "Feedback submitted.");
            if (res.new_goal) {
                setGoal(res.new_goal);
            }
            setFeedback("");
        } catch {
            setStatus("Failed to submit feedback.");
        } finally {
            setLoading(false);
        }
    };

    return (
        <Card className="h-auto mb-4">
            <CardHeader>
                <CardTitle>Feedback Loop</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
                <input
                    value={goal}
                    onChange={(e) => setGoal(e.target.value)}
                    placeholder="Current goal (required)"
                    className="w-full rounded-md bg-white/5 border border-white/10 px-3 py-2 text-sm"
                />
                <textarea
                    value={feedback}
                    onChange={(e) => setFeedback(e.target.value)}
                    placeholder="What should be refined or fixed?"
                    rows={3}
                    className="w-full rounded-md bg-white/5 border border-white/10 px-3 py-2 text-sm"
                />
                <input
                    value={summary}
                    onChange={(e) => setSummary(e.target.value)}
                    placeholder="Optional context / history summary"
                    className="w-full rounded-md bg-white/5 border border-white/10 px-3 py-2 text-sm"
                />
                {status && <div className="text-xs text-muted-foreground">{status}</div>}
                <button
                    onClick={handleSubmit}
                    disabled={loading}
                    className="w-full bg-white/10 hover:bg-white/20 text-sm py-2 rounded transition-colors disabled:opacity-50"
                >
                    {loading ? "Submitting..." : "Submit Feedback"}
                </button>
            </CardContent>
        </Card>
    );
}

function NaturalLanguageAutomationCard() {
    const { data: nlRuns } = useNlRuns(10);
    const { data: nlMetrics } = useNlRunMetrics(50);
    const { data: approvalPolicies, refetch: refetchApprovalPolicies } = useApprovalPolicies(10);
    const [prompt, setPrompt] = useState("");
    const [slotsInput, setSlotsInput] = useState("");
    const [sessionId, setSessionId] = useState<string | null>(null);
    const [planId, setPlanId] = useState<string | null>(null);
    const [intent, setIntent] = useState<string | null>(null);
    const [confidence, setConfidence] = useState<number | null>(null);
    const [missingSlots, setMissingSlots] = useState<string[]>([]);
    const [followUp, setFollowUp] = useState<string | null>(null);
    const [planSteps, setPlanSteps] = useState<string[]>([]);
    const [execStatus, setExecStatus] = useState<string | null>(null);
    const [execLogs, setExecLogs] = useState<string[]>([]);
    const [verifyStatus, setVerifyStatus] = useState<string | null>(null);
    const [verifyIssues, setVerifyIssues] = useState<string[]>([]);
    const [approveAction, setApproveAction] = useState("open_booking_link");
    const [approvalDecision, setApprovalDecision] = useState("allow_once");
    const [approvalRisk, setApprovalRisk] = useState<string | null>(null);
    const [approvalPolicy, setApprovalPolicy] = useState<string | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [loading, setLoading] = useState(false);
    const [history, setHistory] = useState<Array<{ time: string; prompt: string; status: string }>>([]);
    const [summary, setSummary] = useState<string | null>(null);
    const [approvalHistory, setApprovalHistory] = useState<Array<{ time: string; action: string; result: string }>>([]);
    const [historyFilter, setHistoryFilter] = useState("");
    const [executionProfile, setExecutionProfile] = useState<ExecutionProfile>("strict");
    const [lastRunId, setLastRunId] = useState<string | null>(null);
    const [stageRuns, setStageRuns] = useState<TaskStageRun[]>([]);
    const [stageAssertions, setStageAssertions] = useState<TaskStageAssertion[]>([]);

    const loadRunDiagnostics = async (runId?: string | null) => {
        if (!runId) {
            setStageRuns([]);
            setStageAssertions([]);
            return;
        }
        try {
            const [stages, assertions] = await Promise.all([
                fetchTaskRunStages(runId),
                fetchTaskRunAssertions(runId),
            ]);
            setLastRunId(runId);
            setStageRuns(stages);
            setStageAssertions(assertions);
        } catch (error) {
            console.error("Failed to load dashboard run diagnostics", error);
            setLastRunId(runId);
            setStageRuns([]);
            setStageAssertions([]);
        }
    };

    const parseSlots = (raw: string): Record<string, string> | undefined => {
        if (!raw.trim()) return undefined;
        const parsed = JSON.parse(raw) as Record<string, unknown>;
        const normalized: Record<string, string> = {};
        for (const [key, value] of Object.entries(parsed)) {
            if (value === null || value === undefined) continue;
            normalized[key] = String(value);
        }
        return normalized;
    };

    const handleIntent = async () => {
        if (!prompt.trim()) return;
        setLoading(true);
        setError(null);
        try {
            const res = await agentIntent(prompt.trim());
            setSessionId(res.session_id);
            setIntent(res.intent);
            setConfidence(res.confidence);
            setMissingSlots(res.missing_slots);
            setFollowUp(res.follow_up ?? null);
            setExecStatus(null);
            setExecLogs([]);
            setVerifyStatus(null);
            setVerifyIssues([]);
            setSummary(null);
        } catch {
            setError("Failed to parse intent.");
        } finally {
            setLoading(false);
        }
    };

    const handlePlan = async () => {
        if (!sessionId) {
            setError("Run intent first.");
            return;
        }
        setLoading(true);
        setError(null);
        try {
            let slots: Record<string, string> | undefined;
            try {
                slots = parseSlots(slotsInput);
            } catch {
                setError("Slots JSON is invalid.");
                return;
            }
            const res = await agentPlan(sessionId, slots);
            setPlanId(res.plan_id);
            setPlanSteps(res.steps.map((step) => `${step.step_type}: ${step.description}`));
            setMissingSlots(res.missing_slots);
        } catch {
            setError("Failed to build plan.");
        } finally {
            setLoading(false);
        }
    };

    const handleExecute = async () => {
        if (!planId) {
            setError("Build plan first.");
            return;
        }
        setLoading(true);
        setError(null);
        try {
            const res = await agentExecute(planId, executionProfile);
            setExecStatus(res.status);
            setExecLogs(res.logs);
            setLastRunId(res.run_id ?? null);
            const summaryLine = res.logs.find((line) => line.startsWith("Summary: "));
            setSummary(summaryLine ? summaryLine.replace("Summary: ", "") : null);
            await loadRunDiagnostics(res.run_id);
            setHistory((prev) => [
                { time: format(new Date(), "HH:mm:ss"), prompt: prompt || "(no prompt)", status: res.status },
                ...prev,
            ].slice(0, 5));
        } catch {
            setError("Execution failed.");
        } finally {
            setLoading(false);
        }
    };

    const handleVerify = async () => {
        if (!planId) {
            setError("Build plan first.");
            return;
        }
        setLoading(true);
        setError(null);
        try {
            const res = await agentVerify(planId);
            setVerifyStatus(res.ok ? "OK" : "FAIL");
            setVerifyIssues(res.issues);
        } catch {
            setError("Verification failed.");
        } finally {
            setLoading(false);
        }
    };

    const handleApprove = async () => {
        if (!planId) {
            setError("Build plan first.");
            return;
        }
        if (!approveAction.trim()) {
            setError("Approval action is required.");
            return;
        }
        setLoading(true);
        setError(null);
        try {
            const res = await agentApprove(planId, approveAction.trim(), approvalDecision);
            setExecStatus(res.requires_approval ? "approval_required" : res.status);
            setExecLogs((prev) => [...prev, res.message]);
            setApprovalRisk(res.risk_level);
            setApprovalPolicy(res.policy);
            setApprovalHistory((prev) => [
                {
                    time: format(new Date(), "HH:mm:ss"),
                    action: `${approveAction.trim()} (${approvalDecision})`,
                    result: res.status,
                },
                ...prev,
            ].slice(0, 5));
            if (approvalDecision === "allow_always" || approvalDecision === "deny") {
                await refetchApprovalPolicies();
            }
        } catch {
            setError("Approval failed.");
        } finally {
            setLoading(false);
        }
    };

    const handleRemovePolicy = async (policyKey: string) => {
        try {
            await removeApprovalPolicy(policyKey);
            await refetchApprovalPolicies();
        } catch {
            setError("Failed to remove approval policy.");
        }
    };

    const handleRunAll = async () => {
        if (!prompt.trim()) return;
        setLoading(true);
        setError(null);
        try {
            const intentRes = await agentIntent(prompt.trim());
            setSessionId(intentRes.session_id);
            setIntent(intentRes.intent);
            setConfidence(intentRes.confidence);
            setMissingSlots(intentRes.missing_slots);
            setFollowUp(intentRes.follow_up ?? null);

            let slots: Record<string, string> | undefined;
            try {
                slots = parseSlots(slotsInput);
            } catch {
                setError("Slots JSON is invalid.");
                return;
            }

            const planRes = await agentPlan(intentRes.session_id, slots);
            setPlanId(planRes.plan_id);
            setPlanSteps(planRes.steps.map((step) => `${step.step_type}: ${step.description}`));
            setMissingSlots(planRes.missing_slots);

            const execRes = await agentExecute(planRes.plan_id, executionProfile);
            setExecStatus(execRes.status);
            setExecLogs(execRes.logs);
            setLastRunId(execRes.run_id ?? null);
            const summaryLine = execRes.logs.find((line) => line.startsWith("Summary: "));
            setSummary(summaryLine ? summaryLine.replace("Summary: ", "") : null);
            await loadRunDiagnostics(execRes.run_id);
            setHistory((prev) => [
                { time: format(new Date(), "HH:mm:ss"), prompt: prompt || "(no prompt)", status: execRes.status },
                ...prev,
            ].slice(0, 5));

            const verifyRes = await agentVerify(planRes.plan_id);
            setVerifyStatus(verifyRes.ok ? "OK" : "FAIL");
            setVerifyIssues(verifyRes.issues);
        } catch {
            setError("Run all failed.");
        } finally {
            setLoading(false);
        }
    };

    const handleReset = () => {
        setPrompt("");
        setSlotsInput("");
        setSessionId(null);
        setPlanId(null);
        setIntent(null);
        setConfidence(null);
        setMissingSlots([]);
        setFollowUp(null);
        setPlanSteps([]);
        setExecStatus(null);
        setExecLogs([]);
        setVerifyStatus(null);
        setVerifyIssues([]);
        setSummary(null);
        setLastRunId(null);
        setStageRuns([]);
        setStageAssertions([]);
        setError(null);
        setApprovalHistory([]);
        setApprovalRisk(null);
        setApprovalPolicy(null);
    };

    const failureSummary = (() => {
        const issues = new Set<string>();
        const logText = execLogs.join(" ").toLowerCase();
        if (execStatus === "manual_required") issues.add("수동 입력 필요");
        if (execStatus === "approval_required") issues.add("승인 대기");
        if (execStatus === "blocked") issues.add("정책 차단");
        if (logText.includes("search button not found")) issues.add("검색 버튼 미탐지");
        if (logText.includes("auto fill skipped")) issues.add("필드 매칭 실패");
        if (logText.includes("auto fill failed")) issues.add("자동 입력 실패");
        if (logText.includes("failed to open url")) issues.add("페이지 열기 실패");
        if (verifyStatus === "FAIL") issues.add("검증 실패");
        verifyIssues.forEach((issue) => issues.add(issue));
        return Array.from(issues);
    })();

    const timeline = [
        { label: "Intent", status: sessionId ? "done" : "idle" },
        { label: "Plan", status: planId ? "done" : sessionId ? "pending" : "idle" },
        {
            label: "Execute",
            status: execStatus
                ? execStatus === "completed"
                    ? "done"
                    : execStatus === "approval_required" || execStatus === "manual_required"
                        ? "pending"
                        : "blocked"
                : planId
                    ? "pending"
                    : "idle",
        },
        {
            label: "Verify",
            status: verifyStatus ? (verifyStatus === "OK" ? "done" : "blocked") : execStatus ? "pending" : "idle",
        },
        {
            label: "Approve",
            status: approvalHistory.length > 0 ? "done" : execStatus === "approval_required" ? "pending" : "idle",
        },
    ];

    const statusClass = (status: string) => {
        if (status === "done") return "bg-emerald-500/20 text-emerald-200 border-emerald-400/30";
        if (status === "blocked") return "bg-rose-500/20 text-rose-200 border-rose-400/30";
        if (status === "pending") return "bg-amber-500/20 text-amber-200 border-amber-400/30";
        return "bg-white/5 text-white/60 border-white/10";
    };

    const riskClass = (risk: string | null) => {
        if (!risk) return "text-white/60";
        if (risk === "high") return "text-rose-200";
        if (risk === "medium") return "text-amber-200";
        return "text-emerald-200";
    };

    return (
        <Card className="h-auto mb-4 border-indigo-500/20 bg-indigo-500/5">
            <CardHeader>
                <CardTitle>Natural Language Automation</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
                <input
                    value={prompt}
                    onChange={(e) => setPrompt(e.target.value)}
                    placeholder="예: 3월 10일 서울에서 도쿄 왕복 30만원 이하 항공권 찾아줘"
                    className="w-full rounded-md bg-white/5 border border-white/10 px-3 py-2 text-sm"
                />
                <textarea
                    value={slotsInput}
                    onChange={(e) => setSlotsInput(e.target.value)}
                    placeholder='Slots JSON (optional) e.g. {"from":"ICN","to":"NRT","date_start":"2025-03-10"}'
                    rows={2}
                    className="w-full rounded-md bg-white/5 border border-white/10 px-3 py-2 text-xs"
                />
                <div className="grid grid-cols-3 gap-2">
                    <button
                        onClick={() => setExecutionProfile("strict")}
                        className={`text-[11px] py-1.5 rounded border transition-colors ${
                            executionProfile === "strict"
                                ? "bg-white/20 border-white/30 text-white"
                                : "bg-white/5 border-white/10 text-white/70 hover:bg-white/10"
                        }`}
                    >
                        정확(strict)
                    </button>
                    <button
                        onClick={() => setExecutionProfile("test")}
                        className={`text-[11px] py-1.5 rounded border transition-colors ${
                            executionProfile === "test"
                                ? "bg-white/20 border-white/30 text-white"
                                : "bg-white/5 border-white/10 text-white/70 hover:bg-white/10"
                        }`}
                    >
                        테스트(test)
                    </button>
                    <button
                        onClick={() => setExecutionProfile("fast")}
                        className={`text-[11px] py-1.5 rounded border transition-colors ${
                            executionProfile === "fast"
                                ? "bg-white/20 border-white/30 text-white"
                                : "bg-white/5 border-white/10 text-white/70 hover:bg-white/10"
                        }`}
                    >
                        빠름(fast)
                    </button>
                </div>
                <div className="grid grid-cols-2 gap-2">
                    <button
                        onClick={handleIntent}
                        disabled={loading}
                        className="text-xs py-2 rounded bg-white/10 hover:bg-white/20 transition-colors disabled:opacity-50"
                    >
                        Intent
                    </button>
                    <button
                        onClick={handlePlan}
                        disabled={loading}
                        className="text-xs py-2 rounded bg-white/10 hover:bg-white/20 transition-colors disabled:opacity-50"
                    >
                        Plan
                    </button>
                    <button
                        onClick={handleExecute}
                        disabled={loading}
                        className="text-xs py-2 rounded bg-white/10 hover:bg-white/20 transition-colors disabled:opacity-50"
                    >
                        Execute
                    </button>
                    <button
                        onClick={handleVerify}
                        disabled={loading}
                        className="text-xs py-2 rounded bg-white/10 hover:bg-white/20 transition-colors disabled:opacity-50"
                    >
                        Verify
                    </button>
                </div>
                <div className="grid grid-cols-2 gap-2">
                    <button
                        onClick={handleRunAll}
                        disabled={loading}
                        className="text-xs py-2 rounded bg-white/10 hover:bg-white/20 transition-colors disabled:opacity-50"
                    >
                        Run All
                    </button>
                    <button
                        onClick={handleReset}
                        className="text-xs py-2 rounded bg-white/5 hover:bg-white/10 transition-colors"
                    >
                        Reset
                    </button>
                </div>
                <div className="grid grid-cols-2 gap-2">
                    <button
                        onClick={handleExecute}
                        disabled={loading || !planId}
                        className="text-[11px] py-2 rounded bg-white/10 hover:bg-white/20 transition-colors disabled:opacity-50"
                    >
                        Retry Execute
                    </button>
                    <button
                        onClick={() => setExecLogs([])}
                        className="text-[11px] py-2 rounded bg-white/5 hover:bg-white/10 transition-colors"
                    >
                        Clear Logs
                    </button>
                </div>
                <div className="flex gap-2">
                    <select
                        value={approvalDecision}
                        onChange={(e) => setApprovalDecision(e.target.value)}
                        className="rounded-md bg-white/5 border border-white/10 px-2 py-1 text-[11px]"
                    >
                        <option value="allow_once">Allow once</option>
                        <option value="allow_always">Allow always</option>
                        <option value="deny">Deny</option>
                    </select>
                    <input
                        value={approveAction}
                        onChange={(e) => setApproveAction(e.target.value)}
                        placeholder="Approval action"
                        className="flex-1 rounded-md bg-white/5 border border-white/10 px-2 py-1 text-xs"
                    />
                    <button
                        onClick={handleApprove}
                        disabled={loading}
                        className="text-xs px-3 py-1 rounded bg-white/10 hover:bg-white/20 transition-colors disabled:opacity-50"
                    >
                        Approve
                    </button>
                </div>
                {error && <div className="text-xs text-rose-200">{error}</div>}
                {(approvalRisk || approvalPolicy) && (
                    <div className="text-[11px] text-muted-foreground">
                        Approval risk: <span className={riskClass(approvalRisk)}>{approvalRisk ?? "unknown"}</span>
                        {approvalPolicy ? ` · Policy: ${approvalPolicy}` : ""}
                    </div>
                )}
                {nlMetrics && (
                    <div className="grid grid-cols-3 gap-2 text-[11px]">
                        <div className="rounded-md border border-white/10 bg-white/5 p-2">
                            <div className="text-muted-foreground">Success rate</div>
                            <div className="text-white/80">{nlMetrics.success_rate.toFixed(0)}%</div>
                        </div>
                        <div className="rounded-md border border-white/10 bg-white/5 p-2">
                            <div className="text-muted-foreground">Completed</div>
                            <div className="text-white/80">{nlMetrics.completed}/{nlMetrics.total}</div>
                        </div>
                        <div className="rounded-md border border-white/10 bg-white/5 p-2">
                            <div className="text-muted-foreground">Manual/Approval</div>
                            <div className="text-white/80">
                                {nlMetrics.manual_required + nlMetrics.approval_required}
                            </div>
                        </div>
                    </div>
                )}
                {approvalPolicies && approvalPolicies.length > 0 && (
                    <div className="border border-white/10 rounded-md p-2 text-[11px]">
                        <div className="text-[10px] text-muted-foreground mb-1">Approval policies</div>
                        <div className="space-y-1">
                            {approvalPolicies.map((policy) => (
                                <div key={policy.policy_key} className="flex items-center justify-between gap-2">
                                    <div className="truncate text-muted-foreground">
                                        {policy.policy_key} · {policy.decision}
                                    </div>
                                    <button
                                        onClick={() => handleRemovePolicy(policy.policy_key)}
                                        className="text-[10px] px-2 py-0.5 rounded bg-white/10 hover:bg-white/20 transition-colors"
                                    >
                                        Clear
                                    </button>
                                </div>
                            ))}
                        </div>
                    </div>
                )}
                <div className="grid grid-cols-5 gap-2">
                    {timeline.map((step) => (
                        <div
                            key={step.label}
                            className={`rounded border px-2 py-1 text-[10px] text-center ${statusClass(step.status)}`}
                        >
                            {step.label}
                        </div>
                    ))}
                </div>
                {failureSummary.length > 0 && (
                    <div className="rounded-md border border-amber-400/30 bg-amber-500/10 p-2 text-[11px] text-amber-100">
                        <div className="font-semibold mb-1">실패/차단 요약</div>
                        <div className="space-y-0.5">
                            {failureSummary.slice(0, 5).map((issue) => (
                                <div key={issue}>• {issue}</div>
                            ))}
                        </div>
                    </div>
                )}
                <div className="text-[11px] text-muted-foreground space-y-1">
                    {summary && (
                        <div className="rounded-md border border-white/10 bg-black/20 p-2 text-[11px]">
                            <div className="font-semibold text-white/90 mb-1">Summary</div>
                            <div className="text-white/80 break-words">{summary}</div>
                        </div>
                    )}
                    {intent && (
                        <div>
                            Intent: {intent} · {confidence !== null ? `${(confidence * 100).toFixed(0)}%` : ""}
                        </div>
                    )}
                    {sessionId && <div>Session: {sessionId}</div>}
                    {planId && <div>Plan: {planId}</div>}
                    {missingSlots.length > 0 && (
                        <div>Missing slots: {missingSlots.join(", ")}</div>
                    )}
                    {followUp && <div>Follow-up: {followUp}</div>}
                    {planSteps.length > 0 && (
                        <div className="max-h-24 overflow-y-auto">
                            {planSteps.slice(0, 6).map((step, idx) => (
                                <div key={`${step}-${idx}`}>• {step}</div>
                            ))}
                        </div>
                    )}
                    {execStatus && <div>Execute: {execStatus}</div>}
                    <div>Profile: {executionProfile}</div>
                    {execLogs.length > 0 && (
                        <div className="max-h-24 overflow-y-auto">
                            {execLogs.slice(0, 6).map((line, idx) => (
                                <div key={`${line}-${idx}`}>• {line}</div>
                            ))}
                        </div>
                    )}
                    {lastRunId && (
                        <div className="rounded-md border border-white/10 bg-black/20 p-2 text-[11px]">
                            <div className="font-semibold text-white/90 mb-1">Run diagnostics ({lastRunId})</div>
                            {stageRuns.length > 0 && (
                                <div className="space-y-0.5 text-white/80">
                                    {stageRuns.slice(0, 6).map((stage) => (
                                        <div key={`${stage.stage_name}-${stage.id}`}>
                                            • {stage.stage_order}.{stage.stage_name}={stage.status}
                                        </div>
                                    ))}
                                </div>
                            )}
                            {stageAssertions.filter((a) => !a.passed).length > 0 && (
                                <div className="mt-2 text-amber-200 space-y-0.5">
                                    {stageAssertions
                                        .filter((a) => !a.passed)
                                        .slice(0, 6)
                                        .map((assertion) => (
                                            <div key={`${assertion.id}-${assertion.assertion_key}`}>
                                                • {assertion.stage_name}.{assertion.assertion_key} expected={assertion.expected} actual={assertion.actual}
                                            </div>
                                        ))}
                                </div>
                            )}
                        </div>
                    )}
                    {verifyStatus && <div>Verify: {verifyStatus}</div>}
                    {verifyIssues.length > 0 && (
                        <div className="max-h-20 overflow-y-auto text-amber-200">
                            {verifyIssues.map((issue, idx) => (
                                <div key={`${issue}-${idx}`}>• {issue}</div>
                            ))}
                        </div>
                    )}
                    {approvalHistory.length > 0 && (
                        <div className="mt-2 border-t border-white/10 pt-2">
                            <div className="text-[10px] text-muted-foreground mb-1">Approval history</div>
                            {approvalHistory.map((item, idx) => (
                                <div key={`${item.time}-${idx}`} className="text-[10px] text-muted-foreground">
                                    {item.time} · {item.action} · {item.result}
                                </div>
                            ))}
                        </div>
                    )}
                    {history.length > 0 && (
                        <div className="mt-2 border-t border-white/10 pt-2">
                            <div className="flex items-center justify-between mb-1">
                                <div className="text-[10px] text-muted-foreground">Recent runs</div>
                                <input
                                    value={historyFilter}
                                    onChange={(e) => setHistoryFilter(e.target.value)}
                                    placeholder="Filter..."
                                    className="text-[10px] bg-white/5 border border-white/10 rounded px-2 py-0.5"
                                />
                            </div>
                            {history
                                .filter(item => {
                                    const q = historyFilter.trim().toLowerCase();
                                    if (!q) return true;
                                    return (
                                        item.prompt.toLowerCase().includes(q) ||
                                        item.status.toLowerCase().includes(q)
                                    );
                                })
                                .map((item, idx) => (
                                    <div key={`${item.time}-${idx}`} className="text-[10px] text-muted-foreground">
                                        {item.time} · {item.status} · {item.prompt}
                                    </div>
                                ))}
                        </div>
                    )}
                    {nlRuns && nlRuns.length > 0 && (
                        <div className="mt-2 border-t border-white/10 pt-2">
                            <div className="text-[10px] text-muted-foreground mb-1">Saved runs</div>
                            {nlRuns
                                .filter(run => {
                                    const q = historyFilter.trim().toLowerCase();
                                    if (!q) return true;
                                    return (
                                        run.intent.toLowerCase().includes(q) ||
                                        run.status.toLowerCase().includes(q) ||
                                        (run.summary ? run.summary.toLowerCase().includes(q) : false)
                                    );
                                })
                                .slice(0, 5)
                                .map((run) => (
                                    <div key={run.id} className="text-[10px] text-muted-foreground">
                                        {format(new Date(run.created_at), "HH:mm:ss")} · {run.status} · {run.intent}
                                        {run.summary ? ` · ${run.summary}` : ""}
                                    </div>
                                ))}
                        </div>
                    )}
                </div>
            </CardContent>
        </Card>
    );
}

function ControlCard() {
    const [goal, setGoal] = useState("");
    const [loading, setLoading] = useState(false);
    const [message, setMessage] = useState<string | null>(null);

    const handleExecute = async () => {
        if (!goal.trim()) return;
        setLoading(true);
        setMessage(null);
        try {
            const res = await executeGoal(goal.trim());
            setMessage(res.message || "Goal started.");
            setGoal("");
        } catch {
            setMessage("Failed to start goal.");
        } finally {
            setLoading(false);
        }
    };

    return (
        <Card className="h-auto mb-4 border-indigo-500/20 bg-indigo-500/5">
            <CardHeader>
                <CardTitle>Agent Control</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
                <div className="flex gap-2">
                    <input
                        value={goal}
                        onChange={(e) => setGoal(e.target.value)}
                        placeholder="Enter a new goal to execute..."
                        className="flex-1 rounded-md bg-white/5 border border-white/10 px-3 py-2 text-sm focus:outline-none focus:border-indigo-500/50"
                        onKeyDown={(e) => e.key === "Enter" && handleExecute()}
                    />
                    <button
                        onClick={handleExecute}
                        disabled={loading || !goal.trim()}
                        className="bg-indigo-500/20 hover:bg-indigo-500/30 text-indigo-100 text-sm px-4 py-2 rounded transition-colors disabled:opacity-50 font-medium"
                    >
                        {loading ? "Starting..." : "Execute"}
                    </button>
                </div>
                {message && (
                    <div className="text-xs text-indigo-200 bg-indigo-500/10 p-2 rounded">
                        {message}
                    </div>
                )}
            </CardContent>
        </Card>
    );
}

function BetaActionsCard() {
    const [contextResult, setContextResult] = useState<ContextSelection | null>(null);
    const [contextLoading, setContextLoading] = useState(false);
    const [scanResult, setScanResult] = useState<ProjectScan | null>(null);
    const [scanLoading, setScanLoading] = useState(false);
    const [judgmentResult, setJudgmentResult] = useState<Judgment | null>(null);
    const [judgmentLoading, setJudgmentLoading] = useState(false);

    const handleGetContext = async () => {
        setContextLoading(true);
        try {
            const res = await fetchSelectionContext();
            setContextResult(res);
        } catch {
            setContextResult(null);
        } finally {
            setContextLoading(false);
        }
    };

    const handleScanProject = async () => {
        setScanLoading(true);
        try {
            const res = await scanProject(100);
            setScanResult(res);
        } catch {
            setScanResult(null);
        } finally {
            setScanLoading(false);
        }
    };

    const handleRunJudgment = async () => {
        setJudgmentLoading(true);
        try {
            // Dry run judgment without heavy inputs first
            const res = await runJudgment(undefined, 50);
            setJudgmentResult(res);
        } catch {
            setJudgmentResult(null);
        } finally {
            setJudgmentLoading(false);
        }
    };

    return (
        <Card className="h-auto mb-4 border-indigo-500/20 bg-indigo-500/5">
            <CardHeader>
                <CardTitle>Beta Features (Advanced)</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
                <div className="space-y-2">
                    <div className="text-xs text-muted-foreground">Context Awareness</div>
                    <button
                        onClick={handleGetContext}
                        disabled={contextLoading}
                        className="w-full text-[11px] py-1.5 rounded bg-white/10 hover:bg-white/20 transition-colors disabled:opacity-50"
                    >
                        {contextLoading ? "Getting Selection..." : "Get Selected Text (macOS)"}
                    </button>
                    {contextResult && (
                        <div className="rounded-md border border-white/10 bg-black/20 p-2 text-[11px]">
                            <div className={contextResult.found ? "text-emerald-300" : "text-rose-300"}>
                                {contextResult.found ? "Found Selection" : "No Selection / Error"}
                            </div>
                            {contextResult.text && (
                                <div className="mt-1 text-white/80 line-clamp-3 italic">"{contextResult.text}"</div>
                            )}
                            {contextResult.error && (
                                <div className="mt-1 text-rose-300">{contextResult.error}</div>
                            )}
                        </div>
                    )}
                </div>

                <div className="space-y-2">
                    <div className="text-xs text-muted-foreground">Project Scanner</div>
                    <button
                        onClick={handleScanProject}
                        disabled={scanLoading}
                        className="w-full text-[11px] py-1.5 rounded bg-white/10 hover:bg-white/20 transition-colors disabled:opacity-50"
                    >
                        {scanLoading ? "Scanning..." : "Scan Current Project (Limit 100)"}
                    </button>
                    {scanResult && (
                        <div className="rounded-md border border-white/10 bg-black/20 p-2 text-[11px] space-y-1">
                            <div>Type: <span className="text-indigo-300">{scanResult.project_type}</span></div>
                            <div className="text-muted-foreground">{scanResult.files.length} files found.</div>
                            {Object.keys(scanResult.key_files).length > 0 && (
                                <div className="mt-1 space-y-0.5">
                                    <div className="text-[10px] text-muted-foreground">Key Files:</div>
                                    {Object.entries(scanResult.key_files).slice(0, 3).map(([k, v]) => (
                                        <div key={k} className="flex justify-between">
                                            <span>{k}</span>
                                            <span className="text-white/60 truncate max-w-[100px]">{v}</span>
                                        </div>
                                    ))}
                                </div>
                            )}
                        </div>
                    )}
                </div>

                <div className="space-y-2">
                    <div className="text-xs text-muted-foreground">OODA Loop Judgment</div>
                    <button
                        onClick={handleRunJudgment}
                        disabled={judgmentLoading}
                        className="w-full text-[11px] py-1.5 rounded bg-white/10 hover:bg-white/20 transition-colors disabled:opacity-50"
                    >
                        {judgmentLoading ? "Judging..." : "Run Judgment Logic"}
                    </button>
                    {judgmentResult && (
                        <div className="rounded-md border border-white/10 bg-black/20 p-2 text-[11px] space-y-1">
                            <div className="flex justify-between">
                                <span>Status</span>
                                <span className={judgmentResult.status === "stop" || judgmentResult.status === "replan" ? "text-rose-300" : "text-emerald-300"}>
                                    {judgmentResult.status.toUpperCase()}
                                </span>
                            </div>
                            <div className="flex justify-between">
                                <span>Progress</span>
                                <span className={judgmentResult.no_progress ? "text-rose-300" : "text-emerald-300"}>
                                    {judgmentResult.no_progress ? "Stalled" : "Active"}
                                </span>
                            </div>
                            {judgmentResult.reasons.length > 0 && (
                                <div className="mt-1 text-rose-200">
                                    {judgmentResult.reasons.join(", ")}
                                </div>
                            )}
                        </div>
                    )}
                </div>
            </CardContent>
        </Card>
    );
}

function RoutinesCard() {
    const { data: routines, refetch: refetchRoutines } = useRoutines();

    const handleToggle = async (id: number, enabled: boolean) => {
        try {
            await toggleRoutine(id, enabled);
            refetchRoutines();
        } catch {
            // alert("Failed to toggle routine");
        }
    };

    return (
        <Card className="h-auto mb-4 border-purple-500/20 bg-purple-500/5">
            <CardHeader>
                <CardTitle>Active Routines</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2">
                {routines && routines.length > 0 ? (
                    <div className="space-y-2 max-h-40 overflow-y-auto">
                        {routines.map((routine) => (
                            <div key={routine.id} className="flex items-center justify-between bg-white/5 p-2 rounded">
                                <div className="text-xs">
                                    <div className="font-semibold text-white/90">{routine.name}</div>
                                    <div className="font-mono text-white/60">{routine.cron_expression}</div>
                                </div>
                                <label className="relative inline-flex items-center cursor-pointer">
                                    <input
                                        type="checkbox"
                                        className="sr-only peer"
                                        checked={routine.enabled}
                                        onChange={(e) => handleToggle(routine.id, e.target.checked)}
                                    />
                                    <div className="w-9 h-5 bg-white/10 peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-purple-500"></div>
                                </label>
                            </div>
                        ))}
                    </div>
                ) : (
                    <div className="text-muted-foreground text-xs">No routines configured.</div>
                )}
                <RoutineRunHistory />
            </CardContent>
        </Card>
    );
}

function RoutineRunHistory() {
    const { data: runs } = useRoutineRuns(5);
    if (!runs || runs.length === 0) return null;
    return (
        <div className="space-y-2 mt-2 border-t border-purple-500/10 pt-2">
            <div className="text-xs text-muted-foreground">Recent Activity</div>
            <div className="space-y-1">
                {runs.slice(0, 3).map(run => (
                    <div key={run.id} className="flex justify-between text-[10px] bg-white/5 p-1 rounded">
                        <span className="text-purple-200">{run.routine_name}</span>
                        <div className="flex gap-2">
                            <span className={run.status === "success" ? "text-emerald-300" : "text-rose-300"}>
                                {run.status.toUpperCase()}
                            </span>
                            <span className="text-muted-foreground">
                                {format(new Date(run.started_at), "HH:mm")}
                            </span>
                        </div>
                    </div>
                ))}
            </div>
        </div>
    );
}

function VerificationRunHistory() {
    const { data: runs } = useVerificationRuns(5);
    if (!runs || runs.length === 0) return null;
    return (
        <div className="space-y-2 mt-4 border-t border-white/10 pt-4">
            <div className="text-xs text-muted-foreground">Recent Verification Runs</div>
            <div className="space-y-1">
                {runs.slice(0, 3).map(run => (
                    <div key={run.id} className="flex justify-between text-[10px] bg-white/5 p-1 rounded">
                        <span className={
                            run.status === "success" ? "text-emerald-300" :
                                run.status === "failure" ? "text-rose-300" : "text-amber-300"
                        }>
                            {run.mode?.toUpperCase() ?? "—"}
                        </span>
                        <span className="text-muted-foreground">
                            {format(new Date(run.created_at), "MM/dd HH:mm")}
                        </span>
                    </div>
                ))}
            </div>
        </div>
    );
}
