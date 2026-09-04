"""
Performance benchmarking tool for parser providers.

Benchmarks each provider across a folder of documents, reporting per-document
latency and an aggregate summary table.
"""

import argparse
import json
import time
from pathlib import Path
from typing import Optional

from liteparse_eval.providers import (
    ParserProvider,
    LiteparseProvider,
    MarkItDownProvider,
    OpenDataLoaderProvider,
    PdfInspectorProvider,
    PdfToTextProvider,
    PyMuPDFProvider,
    PyMuPDF4LLMMarkdownProvider,
    PyMuPDF4LLMTextProvider,
    PyPDFProvider,
)

ALL_PROVIDERS = [
    "liteparse",
    "pymupdf",
    "pypdf",
    "markitdown",
    "pdftotext",
    "pymupdf4llm-text",
    "pymupdf4llm-md",
    "opendataloader",
    "pdf-inspector",
]

PROVIDER_MAP = {
    "liteparse": LiteparseProvider,
    "pymupdf": PyMuPDFProvider,
    "pypdf": PyPDFProvider,
    "markitdown": MarkItDownProvider,
    "pdftotext": PdfToTextProvider,
    "pymupdf4llm-text": PyMuPDF4LLMTextProvider,
    "pymupdf4llm-md": PyMuPDF4LLMMarkdownProvider,
    "opendataloader": OpenDataLoaderProvider,
    "pdf-inspector": PdfInspectorProvider,
}


def find_pdfs(directory: Path) -> list[Path]:
    """Find all PDF files in a directory (non-recursive)."""
    return sorted(directory.glob("*.pdf"))


def count_pages(file_path: Path) -> Optional[int]:
    """Return the page count of a PDF, or None if it can't be read."""
    try:
        import pypdf

        return len(pypdf.PdfReader(str(file_path)).pages)
    except Exception:
        return None


def time_extraction(provider: ParserProvider, file_path: Path) -> tuple[float, int]:
    """Time a single extraction, returning (seconds, text_length)."""
    start = time.perf_counter()
    text = provider.extract_text(file_path)
    elapsed = time.perf_counter() - start
    return elapsed, len(text)


def format_table(
    providers: list[str],
    doc_names: list[str],
    # results[provider_name][doc_name] = (seconds, chars) | None (error)
    results: dict[str, dict[str, tuple[float, int] | None]],
    page_counts: dict[str, Optional[int]] | None = None,
) -> str:
    """Build a formatted table string."""
    page_counts = page_counts or {}

    # Annotate each document with its page count, e.g. "457_pages.pdf (457p)".
    def doc_label(doc: str) -> str:
        pages = page_counts.get(doc)
        return f"{doc} ({pages}p)" if pages else doc

    labels = {doc: doc_label(doc) for doc in doc_names}

    # Column widths
    doc_col_w = max(len("Document"), *(len(labels[d]) for d in doc_names)) + 2
    prov_col_w = 14

    # Header
    header = f"{'Document':<{doc_col_w}}"
    for p in providers:
        header += f"  {p:>{prov_col_w}}"
    sep = "-" * len(header)

    lines = [sep, header, sep]

    # Per-document rows
    for doc in doc_names:
        row = f"{labels[doc]:<{doc_col_w}}"
        for p in providers:
            entry = results[p].get(doc)
            if entry is None:
                cell = "ERROR"
            else:
                cell = f"{entry[0]:.3f}s"
            row += f"  {cell:>{prov_col_w}}"
        lines.append(row)

    lines.append(sep)

    # Totals row
    row = f"{'TOTAL':<{doc_col_w}}"
    for p in providers:
        total = 0.0
        has_error = False
        for doc in doc_names:
            entry = results[p].get(doc)
            if entry is None:
                has_error = True
            else:
                total += entry[0]
        cell = f"{total:.3f}s" + ("*" if has_error else "")
        row += f"  {cell:>{prov_col_w}}"
    lines.append(row)

    # Average row
    row = f"{'AVG/doc':<{doc_col_w}}"
    for p in providers:
        times = [results[p][d][0] for d in doc_names if results[p].get(d) is not None]
        if times:
            avg = sum(times) / len(times)
            cell = f"{avg:.3f}s"
        else:
            cell = "N/A"
        row += f"  {cell:>{prov_col_w}}"
    lines.append(row)

    # Per-page row (aggregate: total time over successfully-parsed pages).
    total_known_pages = sum(p for p in page_counts.values() if p)
    if total_known_pages:
        row = f"{'MS/PAGE':<{doc_col_w}}"
        for p in providers:
            secs = 0.0
            pages = 0
            for doc in doc_names:
                entry = results[p].get(doc)
                pc = page_counts.get(doc)
                if entry is not None and pc:
                    secs += entry[0]
                    pages += pc
            cell = f"{secs / pages * 1000:.2f}ms" if pages else "N/A"
            row += f"  {cell:>{prov_col_w}}"
        lines.append(row)

    lines.append(sep)
    return "\n".join(lines)


def run_benchmark(
    input_dir: Path,
    providers: list[str],
    output_path: Optional[Path] = None,
    warmup_runs: int = 10,
) -> dict:
    """
    Benchmark providers across all PDFs in a directory.

    Returns a dict suitable for JSON serialization.
    """
    pdf_files = find_pdfs(input_dir)
    if not pdf_files:
        print(f"No PDF files found in {input_dir}")
        return {}

    doc_names = [f.name for f in pdf_files]
    page_counts = {f.name: count_pages(f) for f in pdf_files}
    total_pages = sum(p for p in page_counts.values() if p)

    print(f"Found {len(pdf_files)} documents ({total_pages} pages) in {input_dir}")
    print(f"Providers: {', '.join(providers)}")
    print()

    # results[provider][doc_name] = (seconds, chars) | None
    results: dict[str, dict[str, tuple[float, int] | None]] = {
        p: {} for p in providers
    }

    for provider_name in providers:
        print(f"[{provider_name}]")
        try:
            provider = PROVIDER_MAP[provider_name]()
        except Exception as e:
            print(f"  Failed to initialize: {e}\n")
            for f in pdf_files:
                results[provider_name][f.name] = None
            continue

        # Warmup runs
        if warmup_runs > 0:
            print(f"  Warming up ({warmup_runs} runs)...")
            for _ in range(warmup_runs):
                for pdf_path in pdf_files:
                    try:
                        provider.extract_text(pdf_path)
                    except Exception:
                        pass

        for pdf_path in pdf_files:
            try:
                elapsed, text_len = time_extraction(provider, pdf_path)
                results[provider_name][pdf_path.name] = (elapsed, text_len)
                print(f"  {pdf_path.name}: {elapsed:.3f}s ({text_len:,} chars)")
            except Exception as e:
                results[provider_name][pdf_path.name] = None
                print(f"  {pdf_path.name}: ERROR - {e}")
        print()

    # Print table
    table = format_table(providers, doc_names, results, page_counts)
    print(table)

    # Build JSON output
    output = {
        "input_dir": str(input_dir),
        "documents": doc_names,
        "page_counts": page_counts,
        "total_pages": total_pages,
        "providers": {},
    }
    for p in providers:
        provider_results = {}
        for doc in doc_names:
            entry = results[p].get(doc)
            if entry is None:
                provider_results[doc] = {"error": True}
            else:
                pc = page_counts.get(doc)
                provider_results[doc] = {
                    "seconds": round(entry[0], 4),
                    "text_length": entry[1],
                    "ms_per_page": round(entry[0] / pc * 1000, 3) if pc else None,
                }
        times = [results[p][d][0] for d in doc_names if results[p].get(d) is not None]
        # Aggregate ms/page over successfully-parsed pages only.
        parsed_secs = sum(
            results[p][d][0]
            for d in doc_names
            if results[p].get(d) is not None and page_counts.get(d)
        )
        parsed_pages = sum(
            page_counts[d]
            for d in doc_names
            if results[p].get(d) is not None and page_counts.get(d)
        )
        output["providers"][p] = {
            "per_document": provider_results,
            "total_seconds": round(sum(times), 4) if times else None,
            "avg_seconds": round(sum(times) / len(times), 4) if times else None,
            "ms_per_page": round(parsed_secs / parsed_pages * 1000, 3)
            if parsed_pages
            else None,
            "num_success": len(times),
            "num_error": len(doc_names) - len(times),
        }

    if output_path:
        with open(output_path, "w") as f:
            json.dump(output, f, indent=2)
        print(f"\nResults saved to: {output_path}")

    return output


def main():
    """CLI entry point for the benchmark tool."""
    parser = argparse.ArgumentParser(
        description="Benchmark parse providers across a folder of PDF documents"
    )
    parser.add_argument(
        "input_dir",
        type=Path,
        help="Directory containing PDF documents to benchmark"
    )
    parser.add_argument(
        "--providers",
        type=str,
        nargs="+",
        choices=ALL_PROVIDERS,
        default=ALL_PROVIDERS,
        help="Parse providers to benchmark (default: all providers)"
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="Path to save JSON results"
    )
    parser.add_argument(
        "--warmup-runs",
        type=int,
        default=5,
        help="Number of warmup runs per provider before timing (default: 5)"
    )

    args = parser.parse_args()

    if not args.input_dir.is_dir():
        print(f"Error: Not a directory: {args.input_dir}")
        return 1

    run_benchmark(
        input_dir=args.input_dir,
        providers=args.providers,
        output_path=args.output,
        warmup_runs=args.warmup_runs,
    )

    return 0


if __name__ == "__main__":
    exit(main())
