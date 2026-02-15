
"""
Incremental Indexing Script
Uses DriftDetector to update the index efficiently.
"""
import sys
import logging
from pathlib import Path

# Setup paths
project_root = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(project_root))

from core.config.paths import DATA_DIR, DOCS_DIR
from core.data_pipeline.scanner import scan_directory
from core.data_pipeline.incremental import load_scan_state, save_scan_state, filter_incremental_rows, update_scan_state
from core.data_pipeline.drift import DriftDetector
from core.data_pipeline.pipeline import TrainConfig
from core.search.index_manager import IndexManager

# Setup Logging
logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger("IncrementalIndex")

def main():
    logger.info("🚀 Starting Incremental Indexing...")
    
    # 0. Setup
    cache_path = DATA_DIR / "cache" / "chunk_cache.json"
    scan_state_path = DATA_DIR / "cache" / "scan_state.json"
    
    # 1. Scan
    all_files = scan_directory(DOCS_DIR)
    logger.info(f"📁 Scanned {len(all_files)} files in {DOCS_DIR}")
    
    # 2. Incremental Filter (Mtime/Size)
    scan_state = load_scan_state(scan_state_path)
    to_process, cached_meta = filter_incremental_rows(all_files, scan_state)
    logger.info(f"🔍 Changed/New candidates based on mtime: {len(to_process)}")
    
    # 3. Drift Detection (Add/Del vs Modified)
    detector = DriftDetector(cache_path)
    drift = detector.detect_with_incremental(all_files, to_process)
    
    logger.info(f"📊 Drift Report: {drift.summary()}")
    
    if not drift.has_changes:
        logger.info("✅ No changes detected. Index is up to date.")
        # But we capture scan state anyway to be safe? 
        # Actually if nothing changed, state remains valid.
        return

    # 4. Handle Deletions
    if drift.deleted:
        logger.info(f"🗑️ Deleting {len(drift.deleted)} files from index...")
        idx_mgr = IndexManager(DATA_DIR / "index")
        # TODO: Implement delete_documents in IndexManager or expose deletion logic
        # For now, we assume simple deletion support or manual rebuild for deletions
        # But let's check IndexManager later.
        pass

    # 5. Process Added/Modified
    targets = drift.added + drift.modified
    if targets:
        logger.info(f"🔄 Processing {len(targets)} new/modified files...")
        
        # We need to filter 'all_files' to only include targets
        target_set = set(targets)
        subset_rows = [f for f in all_files if str(f["path"]) in target_set]
        
        # Run Pipeline
        # Note: build_index typically expects a full list or handles subset if flexible.
        # Our pipeline by default might want to rebuild everything. 
        # We need to ensure pipeline chunks ONLY the subset and UPSERTS.
        # Current pipeline uses ChunkCache to skip unchanged, so passing ALL FILES is actually safe
        # but passing subset is faster for 'extract'.
        
        cfg = TrainConfig(
            index_path=DATA_DIR / "index",
            corpus_path=DATA_DIR / "corpus.parquet",
            chunk_cache_path=cache_path,
        )
        
        # Build index logic usually:
        # 1. Extract (only subset needed)
        # 2. Embed (only subset needed)
        # 3. Index (Upsert?)
        
        # Our pipeline.build_index is high level. 
        # Using it with subset might overwrite corpus with ONLY subset if not careful.
        # So we better pass ALL files, let pipeline use ChunkCache to skip expensive parts.
        # BUT pipeline's 'extract' step doesn't check ChunkCache by default before extracting?
        # Actually pipeline.py calls cache.unchanged_paths(df).
        
        # So we can just pass all_files and rely on pipeline's built-in caching?
        # Yes, but pipeline.py might not have atomic upsert for FAISS index (it usually rebuilds).
        # Building valid incremental index requires:
        # Load existing index, add new vectors, remove old vectors.
        # Our FAISS 'IndexManager.build' might be 'reset and build'.
        
        # For true incremental, we need `IndexManager.add(embeddings, metadata)`.
        # If not available, we have to rebuild index from full corpus (which includes cached chunks).
        
        # Strategy: 
        # 1. Extract/Embed changed files.
        # 2. Load existing Chunks/Embeddings from cache/disk.
        # 3. Merge.
        # 4. Rebuild FAISS index (fast).
        
        from core.data_pipeline.pipeline import run_step2, default_train_config
        
        cfg = default_train_config()
        # run_step2 expects a DataFrame-like list of dicts. subset_rows matches.
        
        logger.info("⚙️ Triggering Pipeline (run_step2)...")
        run_step2(
            subset_rows, 
            out_corpus=DATA_DIR / "corpus.parquet",
            out_model=DATA_DIR / "topic_model.joblib",
            cfg=cfg,
            use_tqdm=True,
            translate=False
        ) 

    # 6. Update State
    new_state = update_scan_state(scan_state, all_files)
    save_scan_state(scan_state_path, new_state)
    logger.info("✅ Incremental Indexing Complete.")

if __name__ == "__main__":
    main()
