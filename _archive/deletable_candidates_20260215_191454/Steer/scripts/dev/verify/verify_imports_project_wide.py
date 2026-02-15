import importlib
import pkgutil
import sys
import os
from pathlib import Path

def check_imports(start_dir):
    root = Path(start_dir)
    sys.path.insert(0, str(root))
    
    errors = []
    checked = 0
    
    # Walk core/ and scripts/
    for base in ["core", "scripts"]:
        path = root / base
        if not path.exists():
            continue
            
        for root_dir, dirs, files in os.walk(path):
            for file in files:
                if file.endswith(".py") and not file.startswith("test_"):
                    # Construct module name
                    rel_path = Path(root_dir) / file
                    try:
                        rel_to_root = rel_path.relative_to(root)
                        # ONLY remove .py from end
                        module_parts = list(rel_to_root.parts)
                        if module_parts[-1].endswith(".py"):
                            module_parts[-1] = module_parts[-1][:-3]
                        
                        module_name = ".".join(module_parts)
                        
                        # Skip special cases
                        if "scripts.runners" in module_name: continue
                        
                        # Debug print
                        print(f"Checking: {rel_path} -> {module_name}")
                        
                        importlib.import_module(module_name)
                        checked += 1
                        print(f"[OK] {module_name}")
                    except Exception as e:
                        print(f"[ERROR] {module_name}: {e}")
                        errors.append((module_name, str(e)))

    print(f"\nChecked {checked} modules.")
    if errors:
        print(f"\nFound {len(errors)} errors:")
        for mod, err in errors:
            print(f" - {mod}: {err}")
        sys.exit(1)
    else:
        print("\nAll imports successful.")
        sys.exit(0)

if __name__ == "__main__":
    check_imports(os.getcwd())
