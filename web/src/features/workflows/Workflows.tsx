import { useRecommendations } from "@/lib/hooks";
import { approveRecommendation, rejectRecommendation } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CheckCircle, XCircle, ExternalLink, Workflow } from "lucide-react";
import { useMutation, useQueryClient } from "@tanstack/react-query";

export default function Workflows() {
    const { data: recommendations, isLoading } = useRecommendations();
    const queryClient = useQueryClient();

    // Mutations


    const approve = useMutation({
        mutationFn: approveRecommendation,
        onSuccess: () => queryClient.invalidateQueries({ queryKey: ["recommendations"] }),
        onError: (error) => alert(`Approve failed: ${error}`),
    });

    const reject = useMutation({
        mutationFn: rejectRecommendation,
        onSuccess: () => queryClient.invalidateQueries({ queryKey: ["recommendations"] }),
        onError: (error) => alert(`Reject failed: ${error}`),
    });

    const n8nEditorBaseUrl = (() => {
        const raw = import.meta.env.VITE_N8N_EDITOR_URL as string | undefined;
        const trimmed = raw?.trim().replace(/\/+$/, "");
        return trimmed || "http://localhost:5678";
    })();

    const resolveWorkflowUrl = (workflowUrl?: string | null, workflowId?: string | null) => {
        const direct = workflowUrl?.trim();
        if (direct) return direct;
        const id = workflowId?.trim();
        if (!id) return null;
        if (id.startsWith("provisioning:")) return null;
        return `${n8nEditorBaseUrl}/workflow/${encodeURIComponent(id)}`;
    };

    return (
        <div className="space-y-6">
            <div className="flex items-center justify-between">
                <h2 className="text-3xl font-bold tracking-tight text-glow">Workflows</h2>
                {/* Potentially add filters here */}
            </div>

            {isLoading ? (
                <div className="glass p-10 text-center text-muted-foreground">Loading workflows...</div>
            ) : recommendations && recommendations.length > 0 ? (
                <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                    {recommendations.map((rec) => (
                        <Card key={rec.id} className="group hover:border-primary/30 transition-all duration-300">
                            <CardHeader className="flex flex-row items-start justify-between space-y-0 pb-2">
                                <div className="space-y-1">
                                    <CardTitle className="text-lg flex items-center gap-2">
                                        <Workflow className="w-4 h-4 text-primary" />
                                        {rec.title}
                                    </CardTitle>
                                    <div className="flex items-center gap-2">
                                        <span className={`text-xs px-2 py-0.5 rounded-full font-mono font-bold
                            ${rec.status === 'pending' ? 'bg-yellow-500/20 text-yellow-500' :
                                                rec.status === 'approved' ? 'bg-green-500/20 text-green-500' :
                                                    rec.status === 'rejected' ? 'bg-red-500/20 text-red-500' : 'bg-gray-500/20 text-gray-400'}
                        `}>
                                            {rec.status.toUpperCase()}
                                        </span>
                                        <span className="text-xs text-muted-foreground">Confidence: {(rec.confidence * 100).toFixed(0)}%</span>
                                    </div>
                                </div>
                            </CardHeader>
                            <CardContent className="space-y-4">
                                <p className="text-sm text-muted-foreground line-clamp-3">
                                    {rec.summary}
                                </p>

                                <div className="flex items-center gap-2 pt-2">
                                    {/* Status Actions */}
                                    {rec.status === 'pending' && (
                                        <>
                                            <button
                                                onClick={() => approve.mutate(rec.id)}
                                                disabled={approve.isPending}
                                                className="flex-1 flex items-center justify-center gap-1.5 py-2 rounded-lg bg-green-500/10 text-green-500 hover:bg-green-500/20 text-sm font-medium transition-colors"
                                            >
                                                <CheckCircle className="w-4 h-4" /> Approve
                                            </button>
                                            <button
                                                onClick={() => reject.mutate(rec.id)}
                                                disabled={reject.isPending}
                                                className="flex-1 flex items-center justify-center gap-1.5 py-2 rounded-lg bg-red-500/10 text-red-500 hover:bg-red-500/20 text-sm font-medium transition-colors"
                                            >
                                                <XCircle className="w-4 h-4" /> Reject
                                            </button>
                                        </>
                                    )}
                                    {rec.status === 'approved' && (
                                        (() => {
                                            const workflowUrl = resolveWorkflowUrl(rec.workflow_url, rec.workflow_id);
                                            if (!workflowUrl) {
                                                return (
                                                    <div className="w-full flex items-center justify-center gap-2 py-2 rounded-lg bg-white/5 text-muted-foreground text-sm font-medium border border-white/10">
                                                        <ExternalLink className="w-4 h-4" /> Preparing workflow...
                                                    </div>
                                                );
                                            }
                                            return (
                                                <a
                                                    href={workflowUrl}
                                                    target="_blank"
                                                    rel="noopener noreferrer"
                                                    className="w-full flex items-center justify-center gap-2 py-2 rounded-lg bg-primary/10 text-primary hover:bg-primary/20 text-sm font-medium transition-colors"
                                                >
                                                    <ExternalLink className="w-4 h-4" /> Open in n8n
                                                </a>
                                            );
                                        })()
                                    )}
                                </div>
                            </CardContent>
                        </Card>
                    ))}
                </div>
            ) : (
                <Card className="p-10 text-center">
                    <div className="text-muted-foreground mb-4">No workflows found. Use chat to generate some!</div>
                </Card>
            )}
        </div>
    );
}
