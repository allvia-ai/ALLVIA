import { useState, useRef, useEffect } from "react";
import { Send, Bot, User, Sparkles, Clock, Zap, X } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { sendChatMessage, createRoutine } from "@/lib/api";
import { cn } from "@/lib/utils";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { motion, AnimatePresence } from "framer-motion";

interface Message {
    role: "user" | "assistant";
    text: string;
    command?: string;
    showRoutineCard?: boolean; // NEW: Flag to show inline routine creation
    routineHint?: string; // Hint from user's message
}

// Keywords that trigger routine creation UI
const ROUTINE_KEYWORDS = ["루틴", "routine", "매일", "매주", "자동화", "automate", "schedule", "remind", "알림", "아침", "morning"];

function detectRoutineIntent(text: string): string | null {
    const lower = text.toLowerCase();
    for (const kw of ROUTINE_KEYWORDS) {
        if (lower.includes(kw)) {
            return text; // Return the original text as hint
        }
    }
    return null;
}

// Inline Routine Card Component
function InlineRoutineCard({ hint, onClose, onCreated, onError }: { hint: string; onClose: () => void; onCreated: () => void; onError: (msg: string) => void }) {
    const [name, setName] = useState("");
    const [cron, setCron] = useState("0 9 * * *");
    const [prompt, setPrompt] = useState(hint);
    const [creating, setCreating] = useState(false);
    const queryClient = useQueryClient();

    const handleCreate = async () => {
        if (!name.trim()) return;
        setCreating(true);
        try {
            await createRoutine(name, cron, prompt);
            queryClient.invalidateQueries({ queryKey: ["routines"] });
            onCreated();
        } catch (e) {
            onError(`루틴 생성 실패: ${e instanceof Error ? e.message : "알 수 없는 오류"}`);
        } finally {
            setCreating(false);
        }
    };

    return (
        <motion.div
            initial={{ opacity: 0, y: 20, scale: 0.95 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -10, scale: 0.95 }}
            className="w-full max-w-sm bg-gradient-to-br from-primary/10 to-purple-500/10 border border-primary/30 rounded-2xl p-4 shadow-lg backdrop-blur-sm"
        >
            <div className="flex items-center justify-between mb-3">
                <div className="flex items-center gap-2 text-primary font-medium">
                    <Clock className="w-4 h-4" />
                    <span>Create Routine</span>
                </div>
                <button onClick={onClose} className="text-muted-foreground hover:text-white transition-colors">
                    <X className="w-4 h-4" />
                </button>
            </div>
            <div className="space-y-3">
                <input
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder="Routine Name (e.g., Morning Briefing)"
                    className="w-full px-3 py-2 rounded-lg bg-black/30 border border-white/10 text-sm focus:border-primary/50 outline-none"
                />
                <div>
                    <label className="text-xs text-muted-foreground mb-1 block">Schedule (Cron)</label>
                    <input
                        value={cron}
                        onChange={(e) => setCron(e.target.value)}
                        className="w-full px-3 py-2 rounded-lg bg-black/30 border border-white/10 text-sm font-mono focus:border-primary/50 outline-none"
                    />
                </div>
                <textarea
                    value={prompt}
                    onChange={(e) => setPrompt(e.target.value)}
                    placeholder="What should this routine do?"
                    rows={2}
                    className="w-full px-3 py-2 rounded-lg bg-black/30 border border-white/10 text-sm focus:border-primary/50 outline-none resize-none"
                />
                <motion.button
                    onClick={handleCreate}
                    disabled={creating || !name.trim()}
                    className="w-full py-2 rounded-lg bg-primary text-primary-foreground text-sm font-medium flex items-center justify-center gap-2 disabled:opacity-50"
                    whileHover={{ scale: 1.02 }}
                    whileTap={{ scale: 0.98 }}
                >
                    <Zap className="w-4 h-4" />
                    {creating ? "Creating..." : "Create Routine"}
                </motion.button>
            </div>
        </motion.div>
    );
}

export default function ChatPanel() {
    const [input, setInput] = useState("");
    const [messages, setMessages] = useState<Message[]>([
        { role: "assistant", text: "Hello! I am AllvIa. Try saying '매일 아침 뉴스 요약 해줘' to create a routine!" }
    ]);
    const [showRoutineCard, setShowRoutineCard] = useState(false);
    const [routineHint, setRoutineHint] = useState("");
    const messagesEndRef = useRef<HTMLDivElement>(null);
    const queryClient = useQueryClient();

    const mutation = useMutation({
        mutationFn: sendChatMessage,
        onSuccess: (data) => {
            const text =
                typeof data.response === "string" && data.response.trim().length > 0
                    ? data.response
                    : "✅ 요청을 받았어요. 한 문장만 더 구체적으로 말해주면 바로 도와줄게요.";
            setMessages(prev => [...prev, { role: "assistant", text, command: data.command }]);
            if (data.command === "analyze_patterns") {
                queryClient.invalidateQueries({ queryKey: ["recommendations"] });
            }
        },
        onError: () => {
            setMessages(prev => [...prev, { role: "assistant", text: "❌ Failed to reach the brain. Is Core running?" }]);
        }
    });

    const handleSend = () => {
        if (!input.trim() || mutation.isPending) return;

        const routineIntent = detectRoutineIntent(input);

        // Add user message
        setMessages(prev => [...prev, { role: "user", text: input }]);

        if (routineIntent) {
            // Show inline routine card instead of just sending to backend
            setRoutineHint(routineIntent);
            setShowRoutineCard(true);
            setMessages(prev => [...prev, {
                role: "assistant",
                text: "📋 Got it! I've prepared a routine creation form for you. Fill in the details below:",
                showRoutineCard: true
            }]);
        } else {
            // Normal chat flow
            mutation.mutate(input);
        }

        setInput("");
    };

    const handleRoutineCreated = () => {
        setShowRoutineCard(false);
        setMessages(prev => [...prev, {
            role: "assistant",
            text: "✅ Routine created successfully! Check the Routines tab to see it."
        }]);
    };

    useEffect(() => {
        messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
    }, [messages, showRoutineCard]);

    return (
        <Card className="h-full flex flex-col border-primary/20 bg-black/40 backdrop-blur-xl">
            <CardHeader className="py-4 border-b border-white/5">
                <CardTitle className="text-lg flex items-center gap-2">
                    <Sparkles className="w-5 h-5 text-primary" />
                    Assistant
                </CardTitle>
            </CardHeader>
            <CardContent className="flex-1 flex flex-col p-0 overflow-hidden">
                {/* Messages Area */}
                <div className="flex-1 overflow-y-auto p-4 space-y-4">
                    {messages.map((msg, idx) => (
                        <div key={idx} className={cn("flex w-full gap-3", msg.role === "user" ? "justify-end" : "justify-start")}>
                            {msg.role === "assistant" && (
                                <div className="w-8 h-8 rounded-full bg-primary/20 flex items-center justify-center shrink-0">
                                    <Bot className="w-4 h-4 text-primary" />
                                </div>
                            )}
                            <div className={cn(
                                "max-w-[80%] rounded-2xl px-4 py-2 text-sm leading-relaxed shadow-sm",
                                msg.role === "user"
                                    ? "bg-primary text-primary-foreground rounded-tr-none"
                                    : "bg-white/10 text-foreground rounded-tl-none border border-white/5"
                            )}>
                                {msg.text.split('\n').map((line, i) => <p key={i} className="min-h-[1.2em]">{line}</p>)}
                                {msg.command && (
                                    <div className="mt-2 pt-2 border-t border-white/10 text-xs font-mono text-muted-foreground">
                                        Executed: {msg.command}
                                    </div>
                                )}
                            </div>
                            {msg.role === "user" && (
                                <div className="w-8 h-8 rounded-full bg-white/10 flex items-center justify-center shrink-0">
                                    <User className="w-4 h-4" />
                                </div>
                            )}
                        </div>
                    ))}

                    {/* Inline Routine Creation Card */}
                    <AnimatePresence>
                        {showRoutineCard && (
                            <div className="flex w-full gap-3 justify-start">
                                <div className="w-8 h-8 rounded-full bg-primary/20 flex items-center justify-center shrink-0">
                                    <Bot className="w-4 h-4 text-primary" />
                                </div>
                                <InlineRoutineCard
                                    hint={routineHint}
                                    onClose={() => setShowRoutineCard(false)}
                                    onCreated={handleRoutineCreated}
                                    onError={(msg) => {
                                        setShowRoutineCard(false);
                                        setMessages(prev => [...prev, { role: "assistant", text: `❌ ${msg}` }]);
                                    }}
                                />
                            </div>
                        )}
                    </AnimatePresence>

                    {mutation.isPending && (
                        <div className="flex w-full gap-3 justify-start">
                            <div className="w-8 h-8 rounded-full bg-primary/20 flex items-center justify-center shrink-0">
                                <Bot className="w-4 h-4 text-primary" />
                            </div>
                            <div className="bg-white/5 rounded-2xl px-4 py-2 rounded-tl-none flex items-center gap-1">
                                <span className="w-1.5 h-1.5 bg-white/50 rounded-full animate-bounce [animation-delay:-0.3s]"></span>
                                <span className="w-1.5 h-1.5 bg-white/50 rounded-full animate-bounce [animation-delay:-0.15s]"></span>
                                <span className="w-1.5 h-1.5 bg-white/50 rounded-full animate-bounce"></span>
                            </div>
                        </div>
                    )}
                    <div ref={messagesEndRef} />
                </div>

                {/* Input Area */}
                <div className="p-4 bg-white/5 border-t border-white/5">
                    <div className="flex gap-2 relative">
                        <input
                            value={input}
                            onChange={(e) => setInput(e.target.value)}
                            onKeyDown={(e) => e.key === "Enter" && handleSend()}
                            placeholder="Try: '매일 아침 뉴스 요약 해줘'"
                            className="flex-1 bg-black/20 border border-white/10 rounded-xl px-4 py-3 text-sm focus:outline-none focus:border-primary/50 transition-colors"
                        />
                        <button
                            onClick={handleSend}
                            disabled={!input.trim() || mutation.isPending}
                            className="absolute right-2 top-2 bottom-2 aspect-square rounded-lg bg-primary text-primary-foreground flex items-center justify-center hover:bg-primary/90 disabled:opacity-50 transition-colors"
                        >
                            <Send className="w-4 h-4" />
                        </button>
                    </div>
                </div>
            </CardContent>
        </Card>
    );
}
