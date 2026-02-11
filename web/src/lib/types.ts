import { z } from "zod";

// System Status Schema
export const SystemStatusSchema = z.object({
    cpu_usage: z.number(),
    memory_used: z.number(),
    memory_total: z.number(),
});

// Log Entry Schema (for recent activity)
export const LogEntrySchema = z.object({
    timestamp: z.string(),
    level: z.string(),
    message: z.string(),
});

// Routine Summary Schema
export const RoutineSchema = z.object({
    id: z.number(),
    name: z.string(),
    cron_expression: z.string(),
    enabled: z.number().or(z.boolean()).transform(v => Boolean(v)),
    next_run: z.string().nullable(),
});

// Recommendation/Workflow Schema
export const RecommendationSchema = z.object({
    id: z.number(),
    title: z.string(),
    summary: z.string(),
    status: z.string(),
    confidence: z.number(),
    evidence: z.array(z.string()).optional(), // [NEW] Explainability
    last_error: z.string().nullable().optional(),
});

export const RecommendationMetricsSchema = z.object({
    total: z.number(),
    approved: z.number(),
    rejected: z.number(),
    failed: z.number(),
    pending: z.number(),
    later: z.number(),
    legacy_other: z.number().optional(),
    approval_rate: z.number(),
    last_created_at: z.string().nullable().optional(),
});

export const ExecApprovalSchema = z.object({
    id: z.string(),
    command: z.string(),
    cwd: z.string().nullable().optional(),
    created_at: z.string(),
    expires_at: z.string(),
    status: z.string(),
    decision: z.string().nullable().optional(),
    resolved_at: z.string().nullable().optional(),
    resolved_by: z.string().nullable().optional(),
});

export const ExecAllowlistSchema = z.object({
    id: z.number(),
    pattern: z.string(),
    cwd: z.string().nullable().optional(),
    created_at: z.string(),
    last_used_at: z.string().nullable().optional(),
    uses_count: z.number(),
});

export const ExecResultSchema = z.object({
    id: z.string(),
    command: z.string(),
    cwd: z.string().nullable().optional(),
    status: z.string(),
    output: z.string().nullable().optional(),
    error: z.string().nullable().optional(),
    created_at: z.string(),
    updated_at: z.string().nullable().optional(),
});

export const RoutineRunSchema = z.object({
    id: z.number(),
    routine_id: z.number(),
    routine_name: z.string().optional(),
    started_at: z.string(),
    finished_at: z.string().nullable().optional(),
    status: z.string(),
    error: z.string().nullable().optional(),
});

export const QualityScoreSchema = z.object({
    overall: z.number(),
    breakdown: z.record(z.string(), z.number()),
    issues: z.array(z.string()),
    strengths: z.array(z.string()),
    recommendation: z.string(),
    summary: z.string(),
});

export const QualityScoreRecordSchema = z.object({
    created_at: z.string(),
    score: QualityScoreSchema,
});

export const ConsistencyIssueSchema = z.object({
    path: z.string(),
    reason: z.string(),
    source: z.string(),
});

export const ConsistencyCheckSchema = z.object({
    ok: z.boolean(),
    issues: z.array(ConsistencyIssueSchema),
    backend_paths: z.array(z.string()),
    frontend_calls: z.array(z.object({
        path: z.string(),
        method: z.string().nullable().optional(),
        source: z.string(),
    })),
    summary: z.string(),
    template: z.string(),
});

export const SemanticIssueSchema = z.object({
    file: z.string(),
    reason: z.string(),
    severity: z.string(),
});

export const SemanticVerificationSchema = z.object({
    ok: z.boolean(),
    issues: z.array(SemanticIssueSchema),
    reason: z.string(),
    template: z.string(),
});

export const PerformanceMetricSchema = z.object({
    name: z.string(),
    value: z.number(),
    threshold: z.number(),
    ok: z.boolean(),
});

export const PerformanceVerificationSchema = z.object({
    ok: z.boolean(),
    metrics: z.array(PerformanceMetricSchema),
    reason: z.string(),
    template: z.string(),
});

export const VisualVerdictSchema = z.object({
    prompt: z.string(),
    ok: z.boolean(),
    response: z.string().nullable().optional(),
});

export const VisualVerifySchema = z.object({
    ok: z.boolean(),
    verdicts: z.array(VisualVerdictSchema),
});

export const RuntimeVerifySchema = z.object({
    backend_started: z.boolean(),
    backend_health: z.boolean(),
    backend_build_ok: z.boolean().nullable().optional(),
    frontend_started: z.boolean(),
    frontend_health: z.boolean(),
    frontend_build_ok: z.boolean().nullable().optional(),
    e2e_passed: z.boolean().nullable().optional(),
    issues: z.array(z.string()),
    logs: z.array(z.string()),
});

export const ReleaseBaselineSchema = z.object({
    created_at: z.string(),
}).passthrough();

export const ReleaseGateSchema = z.object({
    ok: z.boolean(),
    regressions: z.array(z.string()),
    warnings: z.array(z.string()),
    current: z.object({
        created_at: z.string().optional(),
    }).optional(),
    baseline: z.object({
        created_at: z.string().optional(),
    }).optional(),
    template: z.string(),
}).passthrough();

export const VerificationRunSchema = z.object({
    id: z.number(),
    created_at: z.string(),
    kind: z.string(),
    mode: z.string().optional(),
    status: z.string().optional(),
    ok: z.boolean(),
    summary: z.string(),
    details: z.string().nullable().optional(),
});

export const AgentIntentResponseSchema = z.object({
    session_id: z.string(),
    intent: z.string(),
    confidence: z.number(),
    slots: z.record(z.string(), z.string()),
    missing_slots: z.array(z.string()),
    follow_up: z.string().nullable().optional(),
});

export const AgentPlanStepSchema = z.object({
    step_id: z.string(),
    step_type: z.string(),
    description: z.string(),
    data: z.unknown(),
});

export const AgentPlanResponseSchema = z.object({
    plan_id: z.string(),
    intent: z.string(),
    steps: z.array(AgentPlanStepSchema),
    missing_slots: z.array(z.string()),
});

export const AgentExecuteResponseSchema = z.object({
    status: z.string(),
    logs: z.array(z.string()),
    approval: z
        .object({
            action: z.string(),
            message: z.string(),
            risk_level: z.string(),
            policy: z.string(),
        })
        .nullable()
        .optional(),
    manual_steps: z.array(z.string()).optional().default([]),
    resume_from: z.number().optional().nullable(),
});

export const AgentVerifyResponseSchema = z.object({
    ok: z.boolean(),
    issues: z.array(z.string()),
});

export const AgentApproveResponseSchema = z.object({
    status: z.string(),
    requires_approval: z.boolean(),
    message: z.string(),
    risk_level: z.string(),
    policy: z.string(),
});

export const ApprovalPolicySchema = z.object({
    policy_key: z.string(),
    decision: z.string(),
    updated_at: z.string(),
});

export const NLRunMetricsSchema = z.object({
    total: z.number(),
    completed: z.number(),
    manual_required: z.number(),
    approval_required: z.number(),
    blocked: z.number(),
    error: z.number(),
    success_rate: z.number(),
});

export const NLRunSchema = z.object({
    id: z.number(),
    created_at: z.string(),
    intent: z.string(),
    prompt: z.string(),
    status: z.string(),
    summary: z.string().nullable().optional(),
    details: z.string().nullable().optional(),
});

export const ContextSelectionSchema = z.object({
    found: z.boolean(),
    text: z.string(),
    error: z.string().optional(),
});

export const ProjectScanSchema = z.object({
    project_type: z.string(),
    files: z.array(z.string()),
    key_files: z.record(z.string(), z.string()),
});

export const JudgmentSchema = z.object({
    status: z.string(),
    reasons: z.array(z.string()),
    no_progress: z.boolean(),
    project_hash: z.string().nullable().optional(),
    consecutive_no_progress: z.number(),
});

export type SystemStatus = z.infer<typeof SystemStatusSchema>;
export type LogEntry = z.infer<typeof LogEntrySchema>;
export type Routine = z.infer<typeof RoutineSchema>;
export type Recommendation = z.infer<typeof RecommendationSchema>;
export type RecommendationMetrics = z.infer<typeof RecommendationMetricsSchema>;
export type ExecApproval = z.infer<typeof ExecApprovalSchema>;
export type ExecAllowlistEntry = z.infer<typeof ExecAllowlistSchema>;
export type ExecResult = z.infer<typeof ExecResultSchema>;
export type RoutineRun = z.infer<typeof RoutineRunSchema>;
export type QualityScore = z.infer<typeof QualityScoreSchema>;
export type QualityScoreRecord = z.infer<typeof QualityScoreRecordSchema>;
export type ConsistencyCheck = z.infer<typeof ConsistencyCheckSchema>;
export type SemanticVerification = z.infer<typeof SemanticVerificationSchema>;
export type PerformanceVerification = z.infer<typeof PerformanceVerificationSchema>;
export type VisualVerifyResult = z.infer<typeof VisualVerifySchema>;
export type RuntimeVerifyResult = z.infer<typeof RuntimeVerifySchema>;
export type ReleaseBaseline = z.infer<typeof ReleaseBaselineSchema>;
export type ReleaseGate = z.infer<typeof ReleaseGateSchema>;
export type VerificationRun = z.infer<typeof VerificationRunSchema>;
export type AgentIntentResponse = z.infer<typeof AgentIntentResponseSchema>;
export type AgentPlanResponse = z.infer<typeof AgentPlanResponseSchema>;
export type AgentExecuteResponse = z.infer<typeof AgentExecuteResponseSchema>;
export type AgentVerifyResponse = z.infer<typeof AgentVerifyResponseSchema>;
export type AgentApproveResponse = z.infer<typeof AgentApproveResponseSchema>;
export type ApprovalPolicy = z.infer<typeof ApprovalPolicySchema>;
export type NLRunMetrics = z.infer<typeof NLRunMetricsSchema>;
export type NLRun = z.infer<typeof NLRunSchema>;
export type ContextSelection = z.infer<typeof ContextSelectionSchema>;
export type ProjectScan = z.infer<typeof ProjectScanSchema>;
export type Judgment = z.infer<typeof JudgmentSchema>;
