// use kernel::types::{Eip1559Transaction, ExecutionError};
// use rlp::RlpStream;

// fn encode_eip1559_unsigned(tx: &Eip1559Transaction) -> Result<Vec<u8>, ExecutionError> {
//     let mut s = RlpStream::new_list(9);

//     s.append(&tx.chain_id);
//     s.append(&tx.nonce);
//     s.append(&tx.max_priority_fee_per_gas);
//     s.append(&tx.max_fee_per_gas);
//     s.append(&tx.gas_limit);

//     match &tx.to {
//         Some(to) => s.append(to),
//         None => s.append_empty_data(),
//     }

//     s.append(&tx.value);
//     s.append(&tx.data);

//     // accessList
//     s.begin_list(tx.access_list.len());
//     for (addr, keys) in &tx.access_list {
//         s.begin_list(2);
//         s.append(addr);

//         s.begin_list(keys.len());
//         for key in keys {
//             s.append(key);
//         }
//     }

//     Ok(s.out().to_vec())
// }
