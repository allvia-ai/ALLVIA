
import sys
import time
import argparse
from pathlib import Path

# Add project root
project_root = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(project_root))

from core.data_pipeline.scanner import run_scan, ScanConfig

def benchmark(root: Path):
    print(f"Benchmarking scanner on {root}...")
    cfg = ScanConfig(roots=[root])
    
    start_time = time.time()
    results = run_scan(cfg)
    end_time = time.time()
    
    elapsed = end_time - start_time
    count = len(results)
    rate = count / elapsed if elapsed > 0 else 0
    
    print(f"Scanned {count} files in {elapsed:.4f}s")
    print(f"Rate: {rate:.1f} files/sec")
    
if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True)
    args = parser.parse_args()
    
    benchmark(Path(args.root))
