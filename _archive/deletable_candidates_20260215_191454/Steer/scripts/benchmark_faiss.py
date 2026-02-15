
import sys
import time
import argparse
import numpy as np
try:
    import psutil
except ImportError:
    psutil = None
import os
from pathlib import Path

# Add project root
project_root = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(project_root))

from core.search.retriever import VectorIndex

def get_memory_usage():
    if psutil is None:
        return 0.0
    process = psutil.Process(os.getpid())
    return process.memory_info().rss / 1024 / 1024  # MB

def benchmark_faiss(n_vecs: int, dim: int = 384):
    print(f"Benchmarking FAISS with {n_vecs} vectors (dim={dim})...")
    print(f"Initial Memory: {get_memory_usage():.2f} MB")
    
    # Generate random vectors
    embeddings = np.random.rand(n_vecs, dim).astype(np.float32)
    # Normalize
    norm = np.linalg.norm(embeddings, axis=1, keepdims=True)
    embeddings = embeddings / norm
    
    index = VectorIndex()
    
    start_add = time.time()
    
    # v1.1 Batch Optimization
    batch_items = []
    for i in range(n_vecs):
        batch_items.append({
            "path": f"/file_{i}", 
            "ext": ".txt", 
            "embedding": embeddings[i], 
            "preview": f"content {i}", 
            "size": 100, 
            "mtime": 100.0, 
            "ctime": 100.0,
            "owner": "user",
            "tokens": []
        })
    
    index.upsert_batch(batch_items)
    
    # Force build/train of FAISS index logic
    t0 = time.time()
    index._ensure_faiss_index() 
    build_time = time.time() - t0
    
    add_time = time.time() - start_add
    
    print(f"Add+Build Time: {add_time:.4f}s (Build only: {build_time:.4f}s)")
    print(f"Memory after Indexing: {get_memory_usage():.2f} MB")
    
    # Search Benchmark
    query_vec = np.random.rand(1, dim).astype(np.float32)
    start_search = time.time()
    for _ in range(100):
        index._ann_scores(query_vec.reshape(-1), fetch=10)
    avg_search = (time.time() - start_search) / 100
    
    print(f"Avg Search Time (100 iters): {avg_search*1000:.4f} ms")

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--count", type=int, default=10000)
    args = parser.parse_args()
    
    benchmark_faiss(args.count)
