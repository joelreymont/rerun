#!/usr/bin/env python3
"""
Reproduction case for issue #11701: Webviewer does not show large chunks when memory limit is exceeded.

This script demonstrates a bug where the webviewer:
- Allocates memory (visible in memory panel) but doesn't render data
- Shows "Data dropped" messages despite available memory headroom
- Creates "zombie" allocations that aren't properly cleared

To reproduce:
1. Run this script
2. Open the web viewer at the printed URL
3. Observe that memory is allocated but images don't display
4. Note "Data dropped" messages despite claimed available memory

Environment details from original issue:
- Windows 11 Enterprise (Build 22631)
- HP ZBook Fury 16 G10 (13th Gen i7, 32GB RAM)
- Rerun version 0.26.2

GitHub Issue: https://github.com/rerun-io/rerun/issues/11701
"""

from __future__ import annotations

import argparse

import numpy as np
import rerun as rr


def log_image_batch(num_images: int, width: int = 640, height: int = 480) -> None:
    """
    Log a batch of images using send_columns.

    This demonstrates the issue where large batches cause memory to be allocated
    but data doesn't render, with "Data dropped" messages appearing.

    Args:
        num_images: Number of images to generate and log
        width: Width of each image
        height: Height of each image
    """
    print(f"Logging batch of {num_images} images ({width}x{height}x3)")
    print(f"Expected memory usage: ~{num_images * width * height * 3 / 1e9:.2f} GB")

    # Generate random image data
    # Shape: (num_images, height, width, 3)
    images = np.random.randint(0, 255, (num_images, height, width, 3), dtype=np.uint8)

    # Create time sequence for the images
    times = np.arange(num_images)

    # Log using send_columns - this is where the issue manifests
    rr.send_columns(
        "camera/image",
        times=[rr.TimeSequenceColumn("frame", times)],
        components=[rr.components.ImageBufferBatch(images)],
    )

    print(f"Sent {num_images} images via send_columns")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--num-images",
        type=int,
        default=2000,
        help="Number of images to log (default: 2000, ~1.8 GB)",
    )
    parser.add_argument(
        "--memory-limit",
        type=str,
        default="500MB",
        help="Memory limit for gRPC server (default: 500MB)",
    )
    parser.add_argument(
        "--width",
        type=int,
        default=640,
        help="Image width (default: 640)",
    )
    parser.add_argument(
        "--height",
        type=int,
        default=480,
        help="Image height (default: 480)",
    )
    parser.add_argument(
        "--use-batched",
        action="store_true",
        help="Use send_columns_batched() instead of send_columns() to demonstrate the fix",
    )
    parser.add_argument(
        "--batch-size",
        type=int,
        default=100,
        help="Batch size when using --use-batched (default: 100)",
    )
    args = parser.parse_args()

    # Initialize Rerun with gRPC server and memory limit
    print(f"Initializing Rerun with gRPC server (memory limit: {args.memory_limit})")
    rr.init(
        "rerun_example_webviewer_memory_issue_11701",
        spawn=False,
    )

    # Start gRPC server with memory limit
    addr = rr.serve(memory_limit=args.memory_limit)
    print(f"gRPC server started at: {addr}")
    print(f"Open web viewer at: http://app.rerun.io?url={addr}")
    print()

    # Log some initial data to confirm connection works
    print("Logging initial test image...")
    test_image = np.random.randint(0, 255, (args.height, args.width, 3), dtype=np.uint8)
    rr.set_time_sequence("frame", 0)
    rr.log("camera/test_image", rr.Image(test_image))
    print("Initial image logged successfully")
    print()

    # Now log the large batch
    print("=" * 60)
    if args.use_batched:
        print("USING BATCHED APPROACH - Demonstrating the fix")
    else:
        print("STARTING LARGE BATCH - This may trigger the memory issue")
    print("=" * 60)
    print()

    if args.use_batched:
        # Use the new batched API that splits large datasets automatically
        print(f"Using send_columns_batched with batch_size={args.batch_size}")
        images = np.random.randint(0, 255, (args.num_images, args.height, args.width, 3), dtype=np.uint8)
        times = np.arange(args.num_images)

        rr.send_columns_batched(
            "camera/image",
            indexes=[rr.TimeColumn("frame", sequence=times)],
            columns=[rr.components.ImageBufferBatch(images)],
            batch_size=args.batch_size,
        )
        print(f"Successfully sent {args.num_images} images in batches")
    else:
        # Use the original approach that triggers the bug
        log_image_batch(args.num_images, args.width, args.height)

    print()
    print("=" * 60)
    print("BATCH COMPLETE")
    print("=" * 60)
    print()

    if args.use_batched:
        print("With batching: All images should display correctly without memory issues")
    else:
        print("Expected behavior: All images should display OR memory should be freed")
        print("Actual behavior (before fix): Memory allocated but data doesn't render")
        print()
        print("To see the fix in action, run with --use-batched flag")

    print()
    print("Press Ctrl+C to exit...")

    # Keep the server running
    import time

    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        print("\nShutting down...")


if __name__ == "__main__":
    main()
