
import sys
import os

print(f"Python executable: {sys.executable}")
print(f"Python version: {sys.version}")

try:
    import numpy
    print(f"Numpy version: {numpy.__version__}")
except ImportError as e:
    print(f"Numpy import failed: {e}")
except Exception as e:
    print(f"Numpy import error: {e}")

try:
    import torch
    print(f"Torch version: {torch.__version__}")
    print(f"Torch MPS available: {torch.backends.mps.is_available()}")
except ImportError as e:
    print(f"Torch import failed: {e}")
except Exception as e:
    print(f"Torch import error: {e}")

try:
    import sentence_transformers
    print(f"SentenceTransformers version: {sentence_transformers.__version__}")
except ImportError as e:
    print(f"SentenceTransformers import failed: {e}")
except Exception as e:
    print(f"SentenceTransformers import error: {e}")

try:
    from sentence_transformers import SentenceTransformer
    print("SentenceTransformer class imported successfully")
except Exception as e:
    print(f"SentenceTransformer class import error: {e}")

try:
    import joblib
    print(f"Joblib version: {joblib.__version__}")
except Exception as e:
    print(f"Joblib import error: {e}")

print("Done checking imports.")
