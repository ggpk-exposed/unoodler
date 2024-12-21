const url = "https://patch.poecdn.com/3.25.3.4/Bundles2/Metadata/Shrines.bundle.bin";
const response = await fetch(url);
if (response.statusCode != 200) {
  console.log(response.statusCode);
}

const dataview = new DataView(await response.arrayBuffer());
const size = dataview.getInt32(0, true);
const granularity = dataview.getInt32(40, true);
const count = Math.ceil(size/granularity);
const first = dataview.getInt32(60, true);

console.log("expected 5012 actual", size);
console.log("compressed size", dataview.getInt32(4, true));
console.log("head size", dataview.getInt32(8, true));
console.log("block count", count);
console.log("first block size", first);
console.log(`?url=${encodeURIComponent(url)}&offset=${60+4*count}&compressed=${first}&extracted=${size}`);

//uint32 uncompressed_size;             0-3
//uint32 total_payload_size;            4-7
//uint32 head_payload_size;             8-11
//struct head_payload_t {
//    uint32 first_file_encode;         12-15
//    uint32 unk10;                     16-19
//    uint64 uncompressed_size2;        20-27
//    uint64 total_payload_size2;       28-35
//    uint32 block_count;               36-39
//    uint32 uncompressed_block_granularity;    40-43
//    uint32 unk28[4];                  44-59
//    uint32 block_sizes[block_count];  60-(59+blk_cnt*4)
//} head;
