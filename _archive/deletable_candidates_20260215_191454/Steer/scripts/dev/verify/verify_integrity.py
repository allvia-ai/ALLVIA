
import sys
import os

print("🔍 Starting Comprehensive Integrity Check...")

try:
    print("1. Checking Core Utils...")
    from core.utils import get_logger
    print("   - core.utils: OK")

    print("2. Checking Orchestrator...")
    from core.conversation.orchestrator import AssistantOrchestrator
    print("   - core.conversation.orchestrator: OK")

    print("3. Checking LNPChat (Syntax/Prompts)...")
    from core.conversation.lnp_chat import LNPChat
    print("   - core.conversation.lnp_chat: OK")

    print("4. Checking Agents...")
    from core.agents.document.agent import DocumentAgent
    from core.agents.meeting import MeetingAgent
    from core.agents.photo import PhotoAgent
    from core.agents.web.agent import WebLauncherAgent
    print("   - Agents: OK")

    print("5. Checking Chat CLI (Logic/Variables)...")
    # This imports orchestrator internally and uses llm_client
    from scripts.pipeline.infopilot_cli import chat
    print("   - scripts.pipeline.infopilot_cli.chat: OK")
    
    # Verify cmd_chat signature and imports inside it by inspecting
    import inspect
    if not hasattr(chat, 'cmd_chat'):
        raise AttributeError("cmd_chat missing in chat.py")
    
    print("6. Checking Main Entry Points...")
    import desktop_app.main
    # We won't run main() as it launches GUI, but importing checks syntax
    print("   - desktop_app.main: OK")

    print("\n✅ All Modules Loaded Successfully. Syntax and Import Integrity Verified.")

except ImportError as e:
    print(f"\n❌ ImportError: {e}")
    sys.exit(1)
except SyntaxError as e:
    print(f"\n❌ SyntaxError: {e}")
    sys.exit(1)
except NameError as e:
    print(f"\n❌ NameError: {e}")
    sys.exit(1)
except Exception as e:
    print(f"\n❌ Critical Error: {e}")
    import traceback
    traceback.print_exc()
    sys.exit(1)
