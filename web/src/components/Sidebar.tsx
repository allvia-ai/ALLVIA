import { Home, PlayCircle, Settings, FileText, MessageCircle } from "lucide-react";
import { cn } from "@/lib/utils";

const navItems = [
    { icon: Home, label: "Dashboard", id: "dashboard" },
    { icon: PlayCircle, label: "Routines", id: "routines" },
    { icon: FileText, label: "Workflows", id: "workflows" },
    { icon: MessageCircle, label: "Chat", id: "chat" },
    { icon: Settings, label: "Settings", id: "settings" },
];

interface SidebarProps {
    active: string;
    onNavigate: (id: string) => void;
}

export function Sidebar({ active, onNavigate }: SidebarProps) {

    return (
        <aside className="w-64 border-r border-white/5 bg-black/40 backdrop-blur-xl flex flex-col h-full">
            <div className="p-6 flex items-center gap-3">
                <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-slate-800 to-slate-950 border border-cyan-300/40 flex items-center justify-center text-[11px] font-bold tracking-wide">
                    <span className="text-cyan-300">A</span>
                    <span className="text-white/90">I</span>
                </div>
                <h1 className="text-xl font-bold tracking-tight text-white/90">
                    <span className="text-cyan-300 font-extrabold">A</span>llv
                    <span className="text-cyan-300 font-extrabold">I</span>a
                </h1>
            </div>

            <nav className="flex-1 px-4 space-y-2 mt-4">
                {navItems.map((item) => (
                    <button
                        key={item.id}
                        onClick={() => onNavigate(item.id)}
                        className={cn(
                            "w-full flex items-center gap-3 px-4 py-3 rounded-xl transition-all duration-200 group text-sm font-medium",
                            active === item.id
                                ? "bg-primary/20 text-primary shadow-[0_0_20px_rgba(59,130,246,0.3)] border border-primary/20"
                                : "text-muted-foreground hover:bg-white/5 hover:text-white"
                        )}
                    >
                        <item.icon className={cn("w-5 h-5 transition-transform group-hover:scale-110", active === item.id ? "text-primary" : "")} />
                        {item.label}
                    </button>
                ))}
            </nav>

            <div className="p-6">
                <div className="glass rounded-xl p-4 flex items-center gap-3">
                    <div className="w-2 h-2 rounded-full bg-green-500 shadow-[0_0_10px_#22c55e]" />
                    <span className="text-xs font-mono text-muted-foreground">System Online</span>
                </div>
            </div>
        </aside>
    );
}
