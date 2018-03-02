use network::packet::{Packet, SerializedBuffer};

enum RPC {

}

pub struct KeepAlive {
//    stream: SerializedBuffer,
}

impl KeepAlive {
    pub const SVUID : i32 = 2;
}

impl Packet for KeepAlive {
    fn read_params(&mut self, stream: &mut SerializedBuffer, error: bool) {
    }

    fn serialize_to_stream(&self, stream: &mut SerializedBuffer) {
        stream.write_i32(KeepAlive::SVUID);
    }
}

pub struct GetInfo {
    pub name: String
}

impl GetInfo {
    pub const SVUID : i32 = 342834823;
}

impl Packet for GetInfo {
    fn serialize_to_stream(&self, stream: &mut SerializedBuffer) {
        stream.write_i32(GetInfo::SVUID);
        stream.write_string(self.name.clone());
    }

    fn read_params(&mut self, stream: &mut SerializedBuffer, error: bool) {
        self.name = stream.read_string();
    }
}

use model::transaction::Transaction;

/*
    address: [u8; ADDRESS_SIZE],
    attachment_timestamp: u64,
    attachment_timestamp_lower_bound: u64,
    attachment_timestamp_upper_bound: u64,
    branch_transaction: [u8; HASH_SIZE],
    trunk_transaction: [u8; HASH_SIZE],
    bundle: [u8; HASH_SIZE],
    current_index: u32,
    hash: [u8; HASH_SIZE],
    last_index: u32,
    nonce: u64,
    tag: String,
    timestamp: u64,
    value: u32
*/
impl Packet for Transaction {


    fn serialize_to_stream(&self, stream: &mut SerializedBuffer) {
        use std::mem::size_of;
        stream.write_i32(GetInfo::SVUID);
        stream.write_bytes(self.address,ADDRESS_SIZE);
        stream.write_u64(self.attachment_timestamp);
        stream.write_u64(self.attachment_timestamp_lower_bound);
        stream.write_64(self.attachment_timestamp_upper_bound);
        stream.write_bytes(self.branch_transaction,HASH_SIZE);
        stream.write_bytes(self.trunk_transaction,HASH_SIZE);
        stream.write_bytes(self.bundle,HASH_SIZE);
        stream.write_u32(self.current_index);
        stream.write_bytes(self.hash,HASH_SIZE);
        stream.write_u32(self.last_index);
        stream.write_u64(self.nonce);
        stream.write_string(self.tag);
        stream.write_u64(self.timestamp);
        stream.write_u32(self.value);

    }

    fn read_params(&mut self, stream: &mut SerializedBuffer, error: bool) {
        stream.rea(self.address,ADDRESS_SIZE);
    }

}


