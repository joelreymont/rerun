from __future__ import annotations

from typing import TYPE_CHECKING

from datafusion import col

if TYPE_CHECKING:
    from .conftest import ServerInstance


def test_component_filtering(server_instance: ServerInstance) -> None:
    """
    Cover the case where a user specifies a component filter on the client.

    We also support push down filtering to take a `.filter()` on the dataframe gets
    pushed into the query. Verify these both give the same results and that we don't
    get any nulls in that column.
    """
    dataset = server_instance.dataset

    component_path = "/obj2:Points3D:positions"

    filter_on_query = (
        dataset.dataframe_query_view(index="time_1", contents="/**")
        .filter_is_not_null(component_path)
        .df()
        .collect_partitioned()
    )

    filter_on_dataframe = (
        dataset.dataframe_query_view(index="time_1", contents="/**")
        .df()
        .filter(col(component_path).is_not_null())
        .collect_partitioned()
    )

    for outer in filter_on_dataframe:
        for inner in outer:
            column = inner.column(component_path)
            assert column.null_count == 0

    assert filter_on_query == filter_on_dataframe


def test_partition_ordering(server_instance: ServerInstance) -> None:
    dataset = server_instance.dataset

    for time_index in ["time_1", "time_2", "time_3"]:
        streams = (
            dataset.dataframe_query_view(index=time_index, contents="/**")
            .fill_latest_at()
            .df()
            .select("rerun_partition_id", time_index)
            .execute_stream_partitioned()
        )

        prior_partition_ids = set()
        for rb_reader in streams:
            prior_partition = ""
            prior_timestamp = 0
            for rb in iter(rb_reader):
                rb = rb.to_pyarrow()
                for idx in range(rb.num_rows):
                    partition = rb[0][idx].as_py()

                    # Nanosecond timestamps cannot be converted using `as_py()`
                    timestamp = rb[1][idx]
                    timestamp = timestamp.value if hasattr(timestamp, "value") else timestamp.as_py()

                    assert partition >= prior_partition
                    if partition == prior_partition and timestamp is not None:
                        assert timestamp >= prior_timestamp
                    else:
                        assert partition not in prior_partition_ids
                        prior_partition_ids.add(partition)

                    prior_partition = partition
                    if timestamp is not None:
                        prior_timestamp = timestamp


def test_tables_to_arrow_reader(server_instance: ServerInstance) -> None:
    dataset = server_instance.dataset

    for rb in dataset.dataframe_query_view(index="time_1", contents="/**").to_arrow_reader():
        assert rb.num_rows > 0

    for partition_batch in dataset.partition_table().to_arrow_reader():
        assert partition_batch.num_rows > 0

    for table_entry in server_instance.client.table_entries()[0].to_arrow_reader():
        assert table_entry.num_rows > 0


def test_query_view_from_schema(server_instance: ServerInstance) -> None:
    """Verify Our Schema is sufficiently descriptive to extract all contents from dataset."""
    from rerun.dataframe import IndexColumnDescriptor

    dataset = server_instance.dataset

    # TODO(nick): This only works for a single shared index column
    # We should consider if our schema is sufficiently descriptive for
    # multi-indices
    index_column = None
    for entry in dataset.schema():
        if isinstance(entry, IndexColumnDescriptor):
            index_column = entry.name
        else:
            local_index_column = index_column
            if entry.is_static:
                local_index_column = None
            contents = dataset.dataframe_query_view(
                index=local_index_column, contents={entry.entity_path: entry.component}
            ).df()
            assert contents.count() > 0


def test_dataset_schema_comparison_self_consistent(server_instance: ServerInstance) -> None:
    dataset = server_instance.dataset

    schema_0 = dataset.schema()
    schema_1 = dataset.schema()
    set_diff = set(schema_0).symmetric_difference(schema_1)

    assert len(set_diff) == 0, f"Schema iterator is not self-consistent: {set_diff}"
    assert schema_0 == schema_1, "Schema is not self-consistent"


def test_partition_id_filter_pushdown_equality(server_instance: ServerInstance) -> None:
    """
    Test partition_id filter push-down with equality (==) operator.

    Verify that filtering using DataFusion's .filter() with == operator produces
    the same result as using the .filter_partition_id() API method.
    """
    dataset = server_instance.dataset

    # Get all partitions first to pick a specific one
    all_partitions = dataset.dataframe_query_view(index="time_1", contents="/**").df().select("rerun_partition_id").distinct().collect()
    if len(all_partitions) == 0:
        return  # No data to test

    # Pick the first partition
    target_partition = all_partitions[0]["rerun_partition_id"][0].as_py()

    # Filter using API method
    api_filtered = dataset.dataframe_query_view(index="time_1", contents="/**").filter_partition_id(target_partition).df().collect()

    # Filter using DataFusion filter (push-down)
    df_filtered = (
        dataset.dataframe_query_view(index="time_1", contents="/**")
        .df()
        .filter(col("rerun_partition_id") == target_partition)
        .collect()
    )

    assert len(api_filtered) == len(df_filtered), f"Row counts differ: API={len(api_filtered)}, DataFusion={len(df_filtered)}"

    # Verify all rows have the target partition
    for batch in df_filtered:
        partition_col = batch.column("rerun_partition_id")
        for i in range(batch.num_rows):
            assert partition_col[i].as_py() == target_partition


def test_partition_id_filter_pushdown_inequality(server_instance: ServerInstance) -> None:
    """
    Test partition_id filter push-down with inequality (!=) operator.

    Verify that filtering with != operator correctly excludes the specified partition.
    """
    dataset = server_instance.dataset

    # Get all partitions
    all_partitions_result = dataset.dataframe_query_view(index="time_1", contents="/**").df().select("rerun_partition_id").distinct().collect()
    if len(all_partitions_result) == 0:
        return  # No data to test

    all_partitions = {batch["rerun_partition_id"][i].as_py() for batch in all_partitions_result for i in range(batch.num_rows)}

    if len(all_partitions) < 2:
        return  # Need at least 2 partitions for this test

    # Pick the first partition to exclude
    excluded_partition = list(all_partitions)[0]
    expected_partitions = all_partitions - {excluded_partition}

    # Filter using DataFusion != operator
    df_filtered = (
        dataset.dataframe_query_view(index="time_1", contents="/**")
        .df()
        .filter(col("rerun_partition_id") != excluded_partition)
        .collect()
    )

    # Verify excluded partition is not present and all others are
    found_partitions = set()
    for batch in df_filtered:
        partition_col = batch.column("rerun_partition_id")
        for i in range(batch.num_rows):
            partition_id = partition_col[i].as_py()
            assert partition_id != excluded_partition, f"Found excluded partition: {excluded_partition}"
            found_partitions.add(partition_id)

    # Verify we got data from the expected partitions (may not be all if some have no data)
    assert len(found_partitions) > 0, "Should have at least one partition"
    assert excluded_partition not in found_partitions


def test_partition_id_filter_pushdown_in_list(server_instance: ServerInstance) -> None:
    """
    Test partition_id filter push-down with IN operator.

    Verify that filtering with the IN operator correctly includes only the specified partitions.
    """
    dataset = server_instance.dataset

    # Get all partitions
    all_partitions_result = dataset.dataframe_query_view(index="time_1", contents="/**").df().select("rerun_partition_id").distinct().collect()
    if len(all_partitions_result) == 0:
        return  # No data to test

    all_partitions = [batch["rerun_partition_id"][i].as_py() for batch in all_partitions_result for i in range(batch.num_rows)]

    if len(all_partitions) < 2:
        # If less than 2 partitions, just test with what we have
        target_partitions = all_partitions
    else:
        # Pick first 2 partitions
        target_partitions = all_partitions[:2]

    # Filter using API method (as baseline)
    api_filtered = dataset.dataframe_query_view(index="time_1", contents="/**").filter_partition_id(*target_partitions).df().collect()

    # Filter using DataFusion IN operator
    df_filtered = (
        dataset.dataframe_query_view(index="time_1", contents="/**")
        .df()
        .filter(col("rerun_partition_id").is_in(target_partitions))
        .collect()
    )

    assert len(api_filtered) == len(df_filtered), f"Row counts differ: API={len(api_filtered)}, DataFusion={len(df_filtered)}"

    # Verify all rows have one of the target partitions
    found_partitions = set()
    for batch in df_filtered:
        partition_col = batch.column("rerun_partition_id")
        for i in range(batch.num_rows):
            partition_id = partition_col[i].as_py()
            assert partition_id in target_partitions, f"Found unexpected partition: {partition_id}"
            found_partitions.add(partition_id)


def test_partition_id_filter_pushdown_not_in_list(server_instance: ServerInstance) -> None:
    """
    Test partition_id filter push-down with NOT IN operator.

    Verify that filtering with the NOT IN operator correctly excludes the specified partitions.
    """
    dataset = server_instance.dataset

    # Get all partitions
    all_partitions_result = dataset.dataframe_query_view(index="time_1", contents="/**").df().select("rerun_partition_id").distinct().collect()
    if len(all_partitions_result) == 0:
        return  # No data to test

    all_partitions = [batch["rerun_partition_id"][i].as_py() for batch in all_partitions_result for i in range(batch.num_rows)]

    if len(all_partitions) < 3:
        return  # Need at least 3 partitions for meaningful test

    # Exclude first 2 partitions
    excluded_partitions = all_partitions[:2]
    expected_partitions = set(all_partitions) - set(excluded_partitions)

    # Filter using DataFusion NOT IN operator
    df_filtered = (
        dataset.dataframe_query_view(index="time_1", contents="/**")
        .df()
        .filter(~col("rerun_partition_id").is_in(excluded_partitions))
        .collect()
    )

    # Verify excluded partitions are not present
    found_partitions = set()
    for batch in df_filtered:
        partition_col = batch.column("rerun_partition_id")
        for i in range(batch.num_rows):
            partition_id = partition_col[i].as_py()
            assert partition_id not in excluded_partitions, f"Found excluded partition: {partition_id}"
            found_partitions.add(partition_id)

    # Verify we got data from expected partitions
    assert len(found_partitions) > 0, "Should have at least one partition"
    for excluded in excluded_partitions:
        assert excluded not in found_partitions


def test_partition_id_filter_combined_with_api(server_instance: ServerInstance) -> None:
    """
    Test that partition_id filter push-down works correctly when combined with API-level filtering.

    When both API .filter_partition_id() and DataFusion .filter() are used, the intersection should be applied.
    """
    dataset = server_instance.dataset

    # Get all partitions
    all_partitions_result = dataset.dataframe_query_view(index="time_1", contents="/**").df().select("rerun_partition_id").distinct().collect()
    if len(all_partitions_result) == 0:
        return  # No data to test

    all_partitions = [batch["rerun_partition_id"][i].as_py() for batch in all_partitions_result for i in range(batch.num_rows)]

    if len(all_partitions) < 2:
        return  # Need at least 2 partitions

    # Use API to select first partition, then use DataFusion to filter for the same one
    target_partition = all_partitions[0]

    # This should work: intersection of {target_partition} and {target_partition} = {target_partition}
    result = (
        dataset.dataframe_query_view(index="time_1", contents="/**")
        .filter_partition_id(target_partition)
        .df()
        .filter(col("rerun_partition_id") == target_partition)
        .collect()
    )

    # Verify we got results
    assert len(result) > 0, "Should have results for the target partition"

    # Verify all rows have the target partition
    for batch in result:
        partition_col = batch.column("rerun_partition_id")
        for i in range(batch.num_rows):
            assert partition_col[i].as_py() == target_partition
