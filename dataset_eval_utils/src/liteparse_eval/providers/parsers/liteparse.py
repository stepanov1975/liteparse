from pathlib import Path
from typing import Optional

from liteparse import LiteParse

from .base import ParserProvider


class LiteparseProvider(ParserProvider):
    """
    Parser provider using the liteparse Python wrapper.

    This provider uses the liteparse library for PDF text extraction.
    """

    def __init__(
        self,
        ocr_enabled: bool = False,
        ocr_server_url: Optional[str] = None,
        ocr_language: str = "en",
        max_pages: int = 1000,
        dpi: int = 150,
        preserve_very_small_text: bool = False,
    ):
        """
        Initialize the liteparse provider.

        Args:
            ocr_enabled: Whether to enable OCR for scanned documents
            ocr_server_url: URL of HTTP OCR server (uses Tesseract if not provided)
            ocr_language: Language code for OCR (e.g., "en", "fr", "de")
            max_pages: Maximum number of pages to parse
            dpi: DPI for rendering (affects OCR quality)
            preserve_very_small_text: Whether to preserve very small text
            cli_path: Custom path to liteparse CLI (auto-detected if not provided)
        """
        self.parser = LiteParse(
            ocr_enabled=ocr_enabled,
            ocr_server_url=ocr_server_url,
            ocr_language=ocr_language,
            max_pages=max_pages,
            dpi=dpi,
            preserve_very_small_text=preserve_very_small_text,
            output_format="markdown",
            quiet=True,
        )

    def extract_text(self, file_path: Path) -> str:
        """Extract markdown from a document using liteparse."""
        result = self.parser.parse(file_path)
        return result.text
