
import os
import random
import string
import argparse
import time
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor

def generate_file(path: Path, min_size: int = 100, max_size: int = 1000):
    """Creates a file with random text content."""
    size = random.randint(min_size, max_size)
    # Generate random text content efficiently
    content = " ".join(["test"] * (size // 5))
    path.write_text(content, encoding="utf-8")

def create_structure(base_dir: Path, total_files: int, depth: int, width: int):
    """
    Generate directory structure.
    Total files are distributed across the tree.
    """
    print(f"Generating {total_files} files in {base_dir} (Depth: {depth}, Width: {width})...")
    
    dirs_to_create = [base_dir]
    created_files = 0
    
    # Create directories first
    current_level_dirs = [base_dir]
    for _ in range(depth):
        next_level_dirs = []
        for d in current_level_dirs:
            for i in range(width):
                new_dir = d / f"dir_{i}"
                new_dir.mkdir(parents=True, exist_ok=True)
                next_level_dirs.append(new_dir)
        dirs_to_create.extend(next_level_dirs)
        current_level_dirs = next_level_dirs
        
    print(f"Created {len(dirs_to_create)} directories.")

    # Distribute files
    files_per_dir = total_files // len(dirs_to_create)
    remainder = total_files % len(dirs_to_create)
    
    def process_dir(directory, count):
        for i in range(count):
            ext = random.choice([".txt", ".md", ".json", ".log"])
            fname = f"file_{random.randint(100000, 999999)}_{i}{ext}"
            generate_file(directory / fname)
            
    with ThreadPoolExecutor(max_workers=os.cpu_count() or 4) as executor:
        futures = []
        for i, d in enumerate(dirs_to_create):
            count = files_per_dir + (1 if i < remainder else 0)
            if count > 0:
                futures.append(executor.submit(process_dir, d, count))
        
        # Wait for completion
        for f in futures:
            f.result()
            
    print("Done.")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Generate synthetic corpus for stress testing.")
    parser.add_argument("--out", required=True, help="Output directory")
    parser.add_argument("--count", type=int, default=10000, help="Total number of files")
    parser.add_argument("--depth", type=int, default=3, help="Directory tree depth")
    parser.add_argument("--width", type=int, default=5, help="Subdirectories per directory")
    
    args = parser.parse_args()
    
    start = time.time()
    create_structure(Path(args.out), args.count, args.depth, args.width)
    elapsed = time.time() - start
    print(f"Generated {args.count} files in {elapsed:.2f}s ({args.count/elapsed:.1f} files/s)")
