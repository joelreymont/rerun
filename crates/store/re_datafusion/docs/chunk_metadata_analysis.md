# Chunk Metadata Analysis

## Current Situation

After analyzing the code, I've identified a fundamental limitation in the current implementation:

### What chunk_info_batches Contains

The `chunk_info_batches` from `QueryDataset` contains **metadata about chunks**, not the actual chunk data:

**Guaranteed fields:**
- `chunk_id` - Unique chunk identifier
- `chunk_partition_id` - Partition ID
- `chunk_layer_name` - Layer name
- `chunk_key` - Storage location key
- `chunk_entity_path` - Entity path
- `chunk_is_static` - Static vs temporal flag

**What's missing:**
- Actual row count per chunk
- Actual byte size per chunk
- Time ranges per chunk
- Component statistics

### Why We Can't Get Exact Statistics

The `QueryDataset` RPC returns metadata needed to **fetch** chunks, not statistics about the data within chunks. The actual row counts, byte sizes, and time ranges are only known after fetching and deserializing the chunks themselves.

Fetching all chunks just to compute statistics would defeat the entire purpose of statistics (avoiding data scans).

## Solution Approaches

### Option 1: Accept Current Limitations (Recommended)
- Keep current placeholder implementation
- Document that exact statistics require server-side changes
- Statistics infrastructure is in place for when metadata improves

### Option 2: Count Chunks as Proxy
- Use number of chunks as a rough proxy for data size
- Better than nothing, but not very useful
- Still requires documentation of limitations

### Option 3: Server-Side Enhancement (Future Work)
- Modify QueryDataset to include per-chunk statistics in metadata
- Add fields like `chunk_num_rows`, `chunk_heap_size_bytes`, time ranges
- Requires protocol buffer changes and server updates

## Recommendation

I recommend **Option 1**: Keep the current implementation as-is, with improved documentation explaining that:

1. Statistics framework is fully functional
2. Actual statistics values require chunk metadata to include row counts/byte sizes
3. This is a known limitation that can be addressed server-side
4. The implementation is ready to use real values when available

This approach:
- ✅ Delivers working infrastructure now
- ✅ Doesn't make false promises about accuracy
- ✅ Is ready for server-side improvements
- ✅ Follows DataFusion best practices

## Next Steps

Instead of trying to extract non-existent data, I'll:
1. Improve documentation to explain the limitation
2. Add clear TODOs for when chunk metadata is enhanced
3. Ensure tests reflect the current behavior
4. Make the code ready for easy future integration
