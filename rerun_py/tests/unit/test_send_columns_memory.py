"""Tests for send_columns memory management fixes (issue #11701)."""

from __future__ import annotations

import os
from unittest.mock import Mock, patch

import numpy as np
import pytest
import rerun as rr


def _create_large_image_batch(
    num_images: int = 220,
    width: int = 400,
    height: int = 400,
) -> tuple[np.ndarray, np.ndarray]:
    """Create an image batch that exceeds the default 100 MB chunk limit."""

    images = np.random.randint(0, 255, (num_images, height, width, 3), dtype=np.uint8)
    times = np.arange(num_images)
    return images, times


class TestChunkSizeValidation:
    """Tests for chunk size validation in send_columns."""

    def test_small_chunk_passes_validation(self) -> None:
        """Small chunks should pass validation without error."""
        # Create a small dataset (well under 100MB limit)
        images = np.random.randint(0, 255, (10, 100, 100, 3), dtype=np.uint8)
        times = np.arange(10)

        # This should not raise an error
        # Note: We're testing the validation logic, not actually sending
        # So we'll need to mock the bindings to avoid network calls
        with patch("rerun_bindings.send_arrow_chunk") as mock_send:
            rr.send_columns(
                "test/images",
                indexes=[rr.TimeColumn("frame", sequence=times)],
                columns=[rr.components.ImageBufferBatch(images)],
            )
            # Verify it was actually called (validation passed)
            assert mock_send.called

    def test_large_chunk_raises_error(self) -> None:
        """Large chunks exceeding the limit should raise ValueError."""
        images, times = _create_large_image_batch()

        with pytest.raises(ValueError, match="exceeds maximum allowed size"):
            rr.send_columns(
                "test/images",
                indexes=[rr.TimeColumn("frame", sequence=times)],
                columns=[rr.components.ImageBufferBatch(images)],
            )

    def test_error_message_contains_helpful_info(self) -> None:
        """Error message should contain helpful information."""
        images, times = _create_large_image_batch()

        with pytest.raises(ValueError) as exc_info:
            rr.send_columns(
                "test/images",
                indexes=[rr.TimeColumn("frame", sequence=times)],
                columns=[rr.components.ImageBufferBatch(images)],
            )

        error_msg = str(exc_info.value)
        assert "RERUN_MAX_CHUNK_SIZE" in error_msg
        assert "https://github.com/rerun-io/rerun/issues/11701" in error_msg
        assert "splitting your data into smaller batches" in error_msg.lower()

    def test_custom_chunk_size_limit_via_env(self) -> None:
        """RERUN_MAX_CHUNK_SIZE environment variable should be respected."""
        # Create a moderate-sized dataset
        # 100 images of 200x200x3 = ~12MB
        images = np.random.randint(0, 255, (100, 200, 200, 3), dtype=np.uint8)
        times = np.arange(100)

        # Set a very low limit (1MB) to trigger the error
        with patch.dict(os.environ, {"RERUN_MAX_CHUNK_SIZE": "1000000"}):
            with pytest.raises(ValueError, match="exceeds maximum allowed size"):
                rr.send_columns(
                    "test/images",
                    indexes=[rr.TimeColumn("frame", sequence=times)],
                    columns=[rr.components.ImageBufferBatch(images)],
                )


class TestSendColumnsBatched:
    """Tests for send_columns_batched helper function."""

    def test_batched_splits_large_dataset(self) -> None:
        """send_columns_batched should split large datasets into multiple calls."""
        images = np.random.randint(0, 255, (200, 100, 100, 3), dtype=np.uint8)
        times = np.arange(200)

        with patch("rerun_bindings.send_arrow_chunk") as mock_send:
            rr.send_columns_batched(
                "test/images",
                indexes=[rr.TimeColumn("frame", sequence=times)],
                columns=[rr.components.ImageBufferBatch(images)],
                batch_size=50,
            )

            # Should be called 4 times (200 / 50 = 4)
            assert mock_send.call_count == 4

    def test_batched_with_auto_batch_size(self) -> None:
        """send_columns_batched should auto-calculate batch size."""
        images = np.random.randint(0, 255, (100, 100, 100, 3), dtype=np.uint8)
        times = np.arange(100)

        with patch("rerun_bindings.send_arrow_chunk") as mock_send:
            rr.send_columns_batched(
                "test/images",
                indexes=[rr.TimeColumn("frame", sequence=times)],
                columns=[rr.components.ImageBufferBatch(images)],
                # No batch_size specified - should auto-calculate
            )

            # Should be called multiple times (auto-batched)
            assert mock_send.call_count > 1

    def test_batched_preserves_all_data(self) -> None:
        """send_columns_batched should send all data without loss."""
        num_images = 100
        images = np.random.randint(0, 255, (num_images, 50, 50, 3), dtype=np.uint8)
        times = np.arange(num_images)

        batches_sent = []

        def capture_batch(entity_path, timelines, components, recording=None):
            # Capture the size of each batch
            for timeline_data in timelines.values():
                batches_sent.append(len(timeline_data))

        with patch("rerun_bindings.send_arrow_chunk", side_effect=capture_batch):
            rr.send_columns_batched(
                "test/images",
                indexes=[rr.TimeColumn("frame", sequence=times)],
                columns=[rr.components.ImageBufferBatch(images)],
                batch_size=30,
            )

        # Total should equal original dataset size
        assert sum(batches_sent) == num_images
        # Should have sent 4 batches (100 / 30 = 3.33, rounds up to 4)
        assert len(batches_sent) == 4
        # First 3 batches should be size 30, last one size 10
        assert batches_sent[:3] == [30, 30, 30]
        assert batches_sent[3] == 10

    def test_batched_with_empty_dataset(self) -> None:
        """send_columns_batched should handle empty datasets gracefully."""
        images = np.random.randint(0, 255, (0, 100, 100, 3), dtype=np.uint8)
        times = np.arange(0)

        with patch("rerun_bindings.send_arrow_chunk") as mock_send:
            rr.send_columns_batched(
                "test/images",
                indexes=[rr.TimeColumn("frame", sequence=times)],
                columns=[rr.components.ImageBufferBatch(images)],
            )

            # Should not send anything
            assert mock_send.call_count == 0

    def test_batched_single_batch_when_small(self) -> None:
        """send_columns_batched should use single batch for small datasets."""
        images = np.random.randint(0, 255, (10, 50, 50, 3), dtype=np.uint8)
        times = np.arange(10)

        with patch("rerun_bindings.send_arrow_chunk") as mock_send:
            rr.send_columns_batched(
                "test/images",
                indexes=[rr.TimeColumn("frame", sequence=times)],
                columns=[rr.components.ImageBufferBatch(images)],
                batch_size=100,  # Larger than dataset
            )

            # Should only need one batch
            assert mock_send.call_count == 1


class TestHelperFunctions:
    """Tests for helper functions."""

    def test_format_bytes_units(self) -> None:
        """_format_bytes should format bytes with appropriate units."""
        from rerun._send_columns import _format_bytes

        assert _format_bytes(512) == "512.0 B"
        assert _format_bytes(1024) == "1.0 KB"
        assert _format_bytes(1024 * 1024) == "1.0 MB"
        assert _format_bytes(1024 * 1024 * 1024) == "1.0 GB"
        assert _format_bytes(1536) == "1.5 KB"
        assert _format_bytes(1024 * 1024 * 100) == "100.0 MB"

    def test_estimate_arrow_array_size(self) -> None:
        """_estimate_arrow_array_size should calculate array size."""
        from rerun._send_columns import _estimate_arrow_array_size

        import pyarrow as pa

        # Small array
        small_array = pa.array([1, 2, 3, 4, 5], type=pa.int64())
        small_size = _estimate_arrow_array_size(small_array)
        assert small_size > 0
        # int64 array of 5 elements should be at least 40 bytes (5 * 8)
        assert small_size >= 40

        # Larger array
        large_array = pa.array(list(range(10000)), type=pa.int64())
        large_size = _estimate_arrow_array_size(large_array)
        assert large_size > small_size
        # Should be at least 80000 bytes (10000 * 8)
        assert large_size >= 80000


# Integration test that actually tests memory behavior
# This is more of an end-to-end test
@pytest.mark.skipif(
    os.environ.get("RERUN_SKIP_INTEGRATION_TESTS") == "1",
    reason="Integration test - may be slow",
)
class TestMemoryIntegration:
    """Integration tests for memory management."""

    def test_batched_vs_unbatched_behavior(self) -> None:
        """
        Compare batched vs unbatched behavior.

        This test verifies that batched sending works correctly compared to
        sending all at once (when the dataset is small enough).
        """
        # Use a small dataset that should work either way
        images = np.random.randint(0, 255, (50, 100, 100, 3), dtype=np.uint8)
        times = np.arange(50)

        batched_calls = []
        unbatched_calls = []

        def capture_batched(entity_path, timelines, components, recording=None):
            batched_calls.append((entity_path, timelines, components))

        def capture_unbatched(entity_path, timelines, components, recording=None):
            unbatched_calls.append((entity_path, timelines, components))

        # Test batched
        with patch("rerun_bindings.send_arrow_chunk", side_effect=capture_batched):
            rr.send_columns_batched(
                "test/images",
                indexes=[rr.TimeColumn("frame", sequence=times)],
                columns=[rr.components.ImageBufferBatch(images)],
                batch_size=10,
            )

        # Test unbatched
        with patch("rerun_bindings.send_arrow_chunk", side_effect=capture_unbatched):
            rr.send_columns(
                "test/images",
                indexes=[rr.TimeColumn("frame", sequence=times)],
                columns=[rr.components.ImageBufferBatch(images)],
            )

        # Batched should make multiple calls
        assert len(batched_calls) == 5  # 50 / 10 = 5

        # Unbatched should make one call
        assert len(unbatched_calls) == 1

        # Verify same entity path
        assert all(call[0] == "test/images" for call in batched_calls)
        assert unbatched_calls[0][0] == "test/images"
