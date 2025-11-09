from __future__ import annotations

import os
from typing import TYPE_CHECKING, Protocol, TypeVar, overload

import numpy as np
import pyarrow as pa
from typing_extensions import deprecated  # type: ignore[misc, unused-ignore]

import rerun_bindings as bindings

from ._baseclasses import Archetype, ComponentColumn, ComponentDescriptor
from .error_utils import catch_and_log_exceptions
from .time import to_nanos, to_nanos_since_epoch

if TYPE_CHECKING:
    from collections.abc import Iterable
    from datetime import datetime, timedelta

    from .recording_stream import RecordingStream

# Default maximum chunk size: 100 MB.
# This can be overridden via RERUN_MAX_CHUNK_SIZE environment variable (in bytes).
_DEFAULT_MAX_CHUNK_SIZE = 100 * 1024 * 1024


def _get_max_chunk_size() -> int:
    """Return the active maximum chunk size."""

    env_value = os.environ.get("RERUN_MAX_CHUNK_SIZE")
    if env_value is None:
        return _DEFAULT_MAX_CHUNK_SIZE

    try:
        chunk_size = int(env_value)
    except ValueError as exc:
        raise ValueError(
            f"RERUN_MAX_CHUNK_SIZE must be an integer representing the limit in bytes, got {env_value!r}"
        ) from exc

    if chunk_size <= 0:
        raise ValueError(
            f"RERUN_MAX_CHUNK_SIZE must be a positive integer value, got {env_value!r}"
        )

    return chunk_size


def _format_bytes(num_bytes: int) -> str:
    """Format byte count as human-readable string."""
    for unit in ["B", "KB", "MB", "GB"]:
        if num_bytes < 1024.0:
            return f"{num_bytes:.1f} {unit}"
        num_bytes /= 1024.0
    return f"{num_bytes:.1f} TB"


def _estimate_arrow_array_size(array: pa.Array) -> int:
    """
    Estimate the memory size of a PyArrow array in bytes.

    This accounts for all buffers (data, offsets, validity, etc.).
    """
    total = 0
    for buffer in array.buffers():
        if buffer is not None:
            total += buffer.size
    return total


class TimeColumnLike(Protocol):
    """Describes interface for objects that can be converted to a column of rerun time values."""

    def timeline_name(self) -> str:
        """Returns the name of the timeline."""
        ...

    def as_arrow_array(self) -> pa.Array:
        """Returns the name of the component."""
        ...


class TimeColumn(TimeColumnLike):
    """
    A column of index (time) values.

    Columnar equivalent to [`rerun.set_time`][].
    """

    # These overloads ensures that mypy can catch errors that would otherwise not be caught until runtime.
    @overload
    def __init__(self, timeline: str, *, sequence: Iterable[int]) -> None: ...

    @overload
    def __init__(
        self,
        timeline: str,
        *,
        duration: Iterable[int] | Iterable[float] | Iterable[timedelta] | Iterable[np.timedelta64],
    ) -> None: ...

    @overload
    def __init__(
        self,
        timeline: str,
        *,
        timestamp: Iterable[int] | Iterable[float] | Iterable[datetime] | Iterable[np.datetime64],
    ) -> None: ...

    def __init__(
        self,
        timeline: str,
        *,
        sequence: Iterable[int] | None = None,
        duration: Iterable[int] | Iterable[float] | Iterable[timedelta] | Iterable[np.timedelta64] | None = None,
        timestamp: Iterable[int] | Iterable[float] | Iterable[datetime] | Iterable[np.datetime64] | None = None,
    ):
        """
        Create a column of index values.

        There is no requirement of monotonicity. You can move the time backwards if you like.

        You are expected to set exactly ONE of the arguments `sequence`, `duration`, or `timestamp`.
        You may NOT change the type of a timeline, so if you use `duration` for a specific timeline,
        you must only use `duration` for that timeline going forward.

        Parameters
        ----------
        timeline:
            The name of the timeline.
        sequence:
            Used for sequential indices, like `frame_nr`.
            Must be integers.
        duration:
            Used for relative times, like `time_since_start`.
            Must either be in seconds, [`datetime.timedelta`][], or [`numpy.timedelta64`][].
        timestamp:
            Used for absolute time indices, like `capture_time`.
            Must either be in seconds since Unix epoch, [`datetime.datetime`][], or [`numpy.datetime64`][].

        """
        if sum(x is not None for x in (sequence, duration, timestamp)) != 1:
            raise ValueError(
                f"TimeColumn: Exactly one of `sequence`, `duration`, and `timestamp` must be set (timeline='{timeline}')",
            )

        self.timeline = timeline

        if sequence is not None:
            self.times = pa.array(sequence, pa.int64())
        elif duration is not None:
            if isinstance(duration, np.ndarray):
                if np.issubdtype(duration.dtype, np.timedelta64):
                    # Already a timedelta array, just ensure it's in nanoseconds
                    self.times = pa.array(duration.astype("timedelta64[ns]"), pa.duration("ns"))
                elif np.issubdtype(duration.dtype, np.number):
                    # Numeric array that needs conversion to nanoseconds
                    self.times = pa.array((duration * 1e9).astype("timedelta64[ns]"), pa.duration("ns"))
                else:
                    raise TypeError(f"Unsupported numpy array dtype: {duration.dtype}")
            else:
                self.times = pa.array(
                    [np.int64(to_nanos(duration)).astype("timedelta64[ns]") for duration in duration], pa.duration("ns")
                )
        elif timestamp is not None:
            # TODO(zehiko) add back timezone support (#9310)
            if isinstance(timestamp, np.ndarray):
                if np.issubdtype(timestamp.dtype, np.datetime64):
                    # Already a datetime array, just ensure it's in nanoseconds
                    self.times = pa.array(timestamp.astype("datetime64[ns]"), pa.timestamp("ns"))
                elif np.issubdtype(timestamp.dtype, np.number):
                    # Numeric array that needs conversion to nanoseconds
                    self.times = pa.array((timestamp * 1e9).astype("datetime64[ns]"), pa.timestamp("ns"))
                else:
                    raise TypeError(f"Unsupported numpy array dtype: {timestamp.dtype}")
            else:
                self.times = pa.array(
                    [np.int64(to_nanos_since_epoch(timestamp)).astype("datetime64[ns]") for timestamp in timestamp],
                    pa.timestamp("ns"),
                )

    def timeline_name(self) -> str:
        """Returns the name of the timeline."""
        return self.timeline

    def as_arrow_array(self) -> pa.Array:
        return self.times


@deprecated(
    """Use `rr.TimeColumn` instead.
    See: https://www.rerun.io/docs/reference/migration/migration-0-23 for more details.""",
)
class TimeSequenceColumn(TimeColumnLike):
    """
    DEPRECATED: A column of time values that are represented as an integer sequence.

    Columnar equivalent to [`rerun.set_time_sequence`][rerun.set_time_sequence].
    """

    def __init__(self, timeline: str, times: Iterable[int]) -> None:
        """
        Create a column of integer sequence time values.

        Parameters
        ----------
        timeline:
            The name of the timeline.
        times:
            An iterable of integer time values.

        """
        self.timeline = timeline
        self.times = times

    def timeline_name(self) -> str:
        """Returns the name of the timeline."""
        return self.timeline

    def as_arrow_array(self) -> pa.Array:
        return pa.array(self.times, type=pa.int64())


@deprecated(
    """Use `rr.TimeColumn` instead.
    See: https://www.rerun.io/docs/reference/migration/migration-0-23 for more details.""",
)
class TimeSecondsColumn(TimeColumnLike):
    """
    DEPRECATED: A column of time values that are represented as floating point seconds.

    Columnar equivalent to [`rerun.set_time_seconds`][rerun.set_time_seconds].
    """

    def __init__(self, timeline: str, times: Iterable[float]) -> None:
        """
        Create a column of floating point seconds time values.

        Parameters
        ----------
        timeline:
            The name of the timeline.
        times:
            An iterable of floating point second time values.

        """
        self.timeline = timeline
        self.times = times

    def timeline_name(self) -> str:
        """Returns the name of the timeline."""
        return self.timeline

    def as_arrow_array(self) -> pa.Array:
        return pa.array([int(t * 1e9) for t in self.times], type=pa.timestamp("ns"))


@deprecated(
    """Use `rr.TimeColumn` instead.
    See: https://www.rerun.io/docs/reference/migration/migration-0-23 for more details.""",
)
class TimeNanosColumn(TimeColumnLike):
    """
    DEPRECATED: A column of time values that are represented as integer nanoseconds.

    Columnar equivalent to [`rerun.set_time_nanos`][rerun.set_time_nanos].
    """

    def __init__(self, timeline: str, times: Iterable[int]) -> None:
        """
        Create a column of integer nanoseconds time values.

        Parameters
        ----------
        timeline:
            The name of the timeline.
        times:
            An iterable of integer nanosecond time values.

        """
        self.timeline = timeline
        self.times = times

    def timeline_name(self) -> str:
        """Returns the name of the timeline."""
        return self.timeline

    def as_arrow_array(self) -> pa.Array:
        return pa.array(self.times, type=pa.timestamp("ns"))


TArchetype = TypeVar("TArchetype", bound=Archetype)


@catch_and_log_exceptions()
def send_columns(
    entity_path: str,
    indexes: Iterable[TimeColumnLike],
    columns: Iterable[ComponentColumn],
    *,
    recording: RecordingStream | None = None,
    strict: bool | None = None,  # noqa: ARG001 - `strict` handled by `@catch_and_log_exceptions`
) -> None:
    r"""
    Send columnar data to Rerun.

    Unlike the regular `log` API, which is row-oriented, this API lets you submit the data
    in a columnar form. Each `TimeColumnLike` and `ComponentColumn` object represents a column
    of data that will be sent to Rerun. The lengths of all these columns must match, and all
    data that shares the same index across the different columns will act as a single logical row,
    equivalent to a single call to `rr.log()`.

    Note that this API ignores any stateful time set on the log stream via [`rerun.set_time`][].
    Furthermore, this will _not_ inject the default timelines `log_tick` and `log_time` timeline columns.

    Parameters
    ----------
    entity_path:
        Path to the entity in the space hierarchy.

        See <https://www.rerun.io/docs/concepts/entity-path> for more on entity paths.
    indexes:
        The time values of this batch of data. Each `TimeColumnLike` object represents a single column
        of timestamps. You usually want to use [`rerun.TimeColumn`][] for this.
    columns:
        The columns of components to log. Each object represents a single column of data.

        In order to send multiple components per time value, explicitly create a [`ComponentColumn`][rerun.ComponentColumn]
        either by constructing it directly, or by calling the `.columns()` method on an `Archetype` type.
    recording:
        Specifies the [`rerun.RecordingStream`][] to use.
        If left unspecified, defaults to the current active data recording, if there is one.
        See also: [`rerun.init`][], [`rerun.set_global_data_recording`][].
    strict:
        If True, raise exceptions on non-loggable data.
        If False, warn on non-loggable data.
        If None, use the global default from `rerun.strict_mode()`

    """
    expected_length = None

    timelines_args = {}
    for t in indexes:
        timeline_name = t.timeline_name()
        time_column = t.as_arrow_array()
        if expected_length is None:
            expected_length = len(time_column)
        elif len(time_column) != expected_length:
            raise ValueError(
                f"All times and components in a column must have the same length. Expected length: {expected_length} but got: {len(time_column)} for timeline: {timeline_name}",
            )

        timelines_args[timeline_name] = time_column

    columns_args: dict[ComponentDescriptor, pa.Array] = {}
    for component_column in columns:
        component_descr = component_column.component_descriptor()
        arrow_list_array = component_column.as_arrow_array()
        if expected_length is None:
            expected_length = len(arrow_list_array)
        elif len(arrow_list_array) != expected_length:
            raise ValueError(
                f"All times and components in a column must have the same length. Expected length: {expected_length} but got: {len(arrow_list_array)} for component: {component_descr}",
            )

        columns_args[component_descr] = arrow_list_array

    # Validate chunk size to prevent memory issues (issue #11701)
    # Estimate the total size of the chunk
    chunk_size = 0
    for time_array in timelines_args.values():
        chunk_size += _estimate_arrow_array_size(time_array)
    for component_array in columns_args.values():
        chunk_size += _estimate_arrow_array_size(component_array)

    max_chunk_size = _get_max_chunk_size()
    if chunk_size > max_chunk_size:
        raise ValueError(
            f"Chunk size ({_format_bytes(chunk_size)}) exceeds maximum allowed size "
            f"({_format_bytes(max_chunk_size)}). "
            f"This limit prevents memory issues when transmitting large batches. "
            f"Consider splitting your data into smaller batches. "
            f"You can adjust the limit via the RERUN_MAX_CHUNK_SIZE environment variable. "
            f"See https://github.com/rerun-io/rerun/issues/11701 for details."
        )

    bindings.send_arrow_chunk(
        entity_path,
        timelines={t.timeline_name(): t.as_arrow_array() for t in indexes},
        components=columns_args,
        recording=recording.to_native() if recording is not None else None,
    )


@catch_and_log_exceptions()
def send_columns_batched(
    entity_path: str,
    indexes: Iterable[TimeColumnLike],
    columns: Iterable[ComponentColumn],
    *,
    batch_size: int | None = None,
    max_chunk_size: int | None = None,
    recording: RecordingStream | None = None,
    strict: bool | None = None,  # noqa: ARG001 - `strict` handled by `@catch_and_log_exceptions`
) -> None:
    """
    Send columnar data to Rerun in batches to avoid memory issues.

    This is a helper function that automatically splits large datasets into appropriately-sized
    chunks before sending them via `send_columns()`. This prevents memory issues on both the
    server and client side when working with large datasets.

    See `send_columns()` for more details on the columnar API.

    Parameters
    ----------
    entity_path:
        Path to the entity in the space hierarchy.
    indexes:
        The time values of this batch of data. Must support slicing if batch_size is not provided.
    columns:
        The columns of components to log. Must support slicing if batch_size is not provided.
    batch_size:
        Number of rows per batch. If not specified, will be calculated based on max_chunk_size.
    max_chunk_size:
        Maximum size of each chunk in bytes. Defaults to RERUN_MAX_CHUNK_SIZE.
        Only used if batch_size is not provided.
    recording:
        Specifies the recording stream to use.
    strict:
        If True, raise exceptions on non-loggable data.
        If False, warn on non-loggable data.

    Examples
    --------
    ```python
    import numpy as np
    import rerun as rr

    # Generate large dataset
    images = np.random.randint(0, 255, (2000, 480, 640, 3), dtype=np.uint8)
    times = np.arange(2000)

    # Send in batches automatically
    rr.send_columns_batched(
        "camera/images",
        indexes=[rr.TimeColumn("frame", sequence=times)],
        columns=[rr.components.ImageBufferBatch(images)],
        batch_size=100,  # Send 100 images at a time
    )
    ```

    """
    # Convert to lists so we can slice them
    indexes_list = list(indexes)
    columns_list = list(columns)

    # Determine total length from first column
    if columns_list:
        total_length = len(columns_list[0].as_arrow_array())
    elif indexes_list:
        total_length = len(indexes_list[0].as_arrow_array())
    else:
        return  # Nothing to send

    # Calculate batch size if not provided
    if batch_size is None:
        chunk_limit = max_chunk_size if max_chunk_size is not None else _get_max_chunk_size()
        # Estimate size per row by checking first few rows
        test_size = min(10, total_length)
        if test_size > 0:
            test_chunk_size = 0
            for idx in indexes_list:
                arr = idx.as_arrow_array()
                test_chunk_size += _estimate_arrow_array_size(arr[:test_size])
            for col in columns_list:
                arr = col.as_arrow_array()
                test_chunk_size += _estimate_arrow_array_size(arr[:test_size])

            bytes_per_row = test_chunk_size / test_size
            # Use 80% of limit to leave headroom
            batch_size = max(1, int((chunk_limit * 0.8) / bytes_per_row))
        else:
            batch_size = 100  # Fallback

    # Send in batches
    num_batches = (total_length + batch_size - 1) // batch_size

    for i in range(num_batches):
        start_idx = i * batch_size
        end_idx = min(start_idx + batch_size, total_length)

        # Slice each index and column
        batch_indexes = []
        for idx in indexes_list:
            arr = idx.as_arrow_array()
            sliced_arr = arr[start_idx:end_idx]
            # Create a new TimeColumn-like object with the sliced data
            # We need to preserve the timeline name
            timeline_name = idx.timeline_name()

            # Create a simple wrapper that implements TimeColumnLike
            class SlicedTimeColumn:
                def __init__(self, name: str, data: pa.Array) -> None:
                    self._name = name
                    self._data = data

                def timeline_name(self) -> str:
                    return self._name

                def as_arrow_array(self) -> pa.Array:
                    return self._data

            batch_indexes.append(SlicedTimeColumn(timeline_name, sliced_arr))

        batch_columns = []
        for col in columns_list:
            arr = col.as_arrow_array()
            sliced_arr = arr[start_idx:end_idx]
            # Create a new ComponentColumn with the sliced data
            descriptor = col.component_descriptor()

            class SlicedComponentColumn:
                def __init__(self, desc: ComponentDescriptor, data: pa.Array) -> None:
                    self._desc = desc
                    self._data = data

                def component_descriptor(self) -> ComponentDescriptor:
                    return self._desc

                def as_arrow_array(self) -> pa.Array:
                    return self._data

            batch_columns.append(SlicedComponentColumn(descriptor, sliced_arr))

        # Send this batch
        send_columns(
            entity_path,
            indexes=batch_indexes,
            columns=batch_columns,
            recording=recording,
            strict=strict,
        )
