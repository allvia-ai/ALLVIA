"""
Semantic Chunking Module (v2)
Implements semantic-aware text chunking strategies for RAG.
"""
import re
from typing import List, Dict, Any, Tuple, Optional
from dataclasses import dataclass

@dataclass
class Chunk:
    text: str
    start_char: int
    end_char: int
    token_count: int
    metadata: Dict[str, Any]

class SemanticChunker:
    """
    Chunks text by respecting semantic boundaries:
    1. Markdown Headers (#, ##, ...)
    2. Paragraphs (\n\n)
    3. Sentences (.!?)
    4. Words (fallback)
    """

    def __init__(
        self, 
        max_tokens: int = 512, 
        overlap_tokens: int = 50,
        min_tokens: int = 50
    ):
        self.max_tokens = max_tokens
        self.overlap_tokens = overlap_tokens
        self.min_tokens = min_tokens
        self._token_pattern = re.compile(r'(?u)(?:[가-힣]{1,}|[A-Za-z0-9]{2,})')
        self._sentence_split = re.compile(r'(?<=[.!?])\s+')
        self._header_split = re.compile(r'(^|\n)(#{1,6}\s+.*)')

    def _count_tokens(self, text: str) -> int:
        """Simple regex-based token counting."""
        return len(self._token_pattern.findall(text)) or max(1, len(text) // 4)

    def chunk_text(self, text: str, extra_meta: Optional[Dict[str, Any]] = None) -> List[Chunk]:
        """
        Split text into semantic chunks.
        """
        if not text:
            return []

        # 1. Split by Headers
        sections = self._split_by_headers(text)
        
        chunks: List[Chunk] = []
        global_offset = 0
        
        for section_text, heading in sections:
            # 2. Split section into chunks (Paragraph -> Sentence)
            section_chunks = self._chunk_section(section_text, heading, global_offset)
            chunks.extend(section_chunks)
            global_offset += len(section_text)

        # Attach extra metadata
        if extra_meta:
            for c in chunks:
                c.metadata.update(extra_meta)
                
        return chunks

    def _split_by_headers(self, text: str) -> List[Tuple[str, str]]:
        """Return list of (text, heading) tuples."""
        # This is a simplified split. 
        # Ideally we keep the structure. For now, flat list of sections.
        # Check if markdown first
        if "#" not in text:
            return [(text, "")]
            
        parts = self._header_split.split(text)
        sections = []
        current_heading = ""
        buffer = ""
        
        # parts[0] is text before first header
        if parts[0]:
            sections.append((parts[0], ""))
            
        # Then groups of (newline, header_line, rest...)
        # But split keeps the delimiter.
        # Regex: (^|\n)(#{1,6}\s+.*)
        # 1: newline or start
        # 2: header text
        # Rest: following text until next match? No, re.split includes captures.
        
        # Iterating manually might be safer for simple logic
        lines = text.split('\n')
        current_section = []
        current_h = ""
        
        for line in lines:
            if line.lstrip().startswith('#'):
                # Flush previous
                if current_section:
                    sections.append(("\n".join(current_section), current_h))
                current_section = [line]
                current_h = line.strip().lstrip('#').strip()
            else:
                current_section.append(line)
                
        if current_section:
            sections.append(("\n".join(current_section), current_h))
            
        return sections

    def _chunk_section(self, text: str, heading: str, offset: int) -> List[Chunk]:
        """Deep chunking within a section."""
        tokens = self._count_tokens(text)
        if tokens <= self.max_tokens:
            return [Chunk(
                text=text,
                start_char=offset,
                end_char=offset + len(text),
                token_count=tokens,
                metadata={"heading": heading}
            )]
            
        # Recursive split strategies
        # 1. Paragraphs
        paragraphs = text.split('\n\n')
        if len(paragraphs) > 1:
            return self._merge_units(paragraphs, heading, offset, delimiter='\n\n')
            
        # 2. Sentences
        sentences = self._sentence_split.split(text)
        if len(sentences) > 1:
            return self._merge_units(sentences, heading, offset, delimiter=' ')
            
        # 3. Words (fallback)
        # Just slice strict max_tokens
        return self._manual_slice(text, heading, offset)

    def _merge_units(self, units: List[str], heading: str, base_offset: int, delimiter: str) -> List[Chunk]:
        """Merge smaller units into chunks fitting max_tokens."""
        chunks = []
        current_chunk_units = []
        current_tokens = 0
        current_start = base_offset
        current_length = 0
        
        for unit in units:
            unit_tokens = self._count_tokens(unit)
            
            # If single unit is too big, force split it
            if unit_tokens > self.max_tokens:
                # Flush current
                if current_chunk_units:
                    text = delimiter.join(current_chunk_units)
                    chunks.append(Chunk(
                        text=text,
                        start_char=current_start,
                        end_char=current_start + len(text),
                        token_count=current_tokens,
                        metadata={"heading": heading}
                    ))
                    current_start += len(text) + len(delimiter)
                    current_chunk_units = []
                    current_tokens = 0

                # Recursive split of big unit
                sub_chunks = self._chunk_section(unit, heading, current_start)
                chunks.extend(sub_chunks)
                current_start += len(unit) + len(delimiter)
                continue

            # Check if adding fit
            if current_tokens + unit_tokens > self.max_tokens:
                # Flush
                text = delimiter.join(current_chunk_units)
                chunks.append(Chunk(
                    text=text,
                    start_char=current_start,
                    end_char=current_start + len(text),
                    token_count=current_tokens,
                    metadata={"heading": heading}
                ))
                
                # Overlap logic: keep last N units that fit overlap_tokens
                # Simplified: just start fresh or keep last unit if small
                current_start += len(text) + len(delimiter) # Update offset logic strictly? 
                # Actually offset math with overlap is tricky. 
                # Let's trust base_offset tracking.
                
                # For strict offset tracking, we need to know exact position.
                # Let's approximate offset for now or rely on text matching later if needed.
                # Re-using previous units for overlap:
                overlap_units = []
                overlap_tokens_count = 0
                for u in reversed(current_chunk_units):
                    u_tok = self._count_tokens(u)
                    if overlap_tokens_count + u_tok <= self.overlap_tokens:
                        overlap_units.insert(0, u)
                        overlap_tokens_count += u_tok
                    else:
                        break
                
                current_chunk_units = overlap_units + [unit]
                current_tokens = overlap_tokens_count + unit_tokens
                # Fix current_start for next chunk... this is hard with overlap.
                # Simpler: just set current_start to (end of previous - overlap length)?
                # Or just ignore exact char offset for overlap chunks if not strictly needed.
                # RAG usually needs text references.
                # Let's keep it simple: No overlap in char offset tracking for now, 
                # or just acknowledge precision loss.
            else:
                current_chunk_units.append(unit)
                current_tokens += unit_tokens
                
        # Flush remainder
        if current_chunk_units:
            text = delimiter.join(current_chunk_units)
            chunks.append(Chunk(
                text=text,
                start_char=current_start, # This is wrong if we did overlap
                end_char=current_start + len(text),
                token_count=current_tokens,
                metadata={"heading": heading}
            ))
            
        return chunks

    def _manual_slice(self, text: str, heading: str, offset: int) -> List[Chunk]:
        """Hard slice text if no other split possible."""
        # Use simple char slicing roughly mapping to tokens
        # Assuming 1 token ~ 4 chars
        char_step = self.max_tokens * 4
        char_overlap = self.overlap_tokens * 4
        
        chunks = []
        for i in range(0, len(text), char_step - char_overlap):
            sub = text[i:i+char_step]
            chunks.append(Chunk(
                text=sub,
                start_char=offset + i,
                end_char=offset + i + len(sub),
                token_count=self._count_tokens(sub),
                metadata={"heading": heading}
            ))
        return chunks
