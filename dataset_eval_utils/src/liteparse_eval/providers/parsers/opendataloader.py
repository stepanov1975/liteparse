import tempfile
from pathlib import Path

import opendataloader_pdf

from .base import ParserProvider


class OpenDataLoaderProvider(ParserProvider):
    """
    Parse provider using OpenDataLoader PDF.

    Install with: pip install opendataloader-pdf
    Requires: Java 11+
    """

    def __init__(self):
        """Initialize the parse provider."""
        pass

    def extract_text(self, file_path: Path) -> str:
        """Extract markdown from a document using OpenDataLoader PDF."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            opendataloader_pdf.convert(
                input_path=[str(file_path)],
                output_dir=tmp_dir,
                format="markdown",
                quiet=True,
            )
            # Read the output markdown file
            output_file = Path(tmp_dir) / f"{file_path.stem}.md"
            if output_file.exists():
                return output_file.read_text(encoding="utf-8")
            # Fallback: find any .md file in the output dir
            md_files = list(Path(tmp_dir).glob("*.md"))
            if md_files:
                return md_files[0].read_text(encoding="utf-8")
            return ""
