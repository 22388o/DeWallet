use std::any::Any;

use async_trait::async_trait;
use bitcoin::{Address, AddressType};
use bitcoin_hashes::{sha256d, Hash};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use walletd_coin_model::BlockchainConnector;
use crate::BitcoinWallet;
use chrono::prelude::DateTime;
use chrono::Utc;
use walletd_coin_model::CryptoWallet;

use prettytable::Table;
use prettytable::row;
use std::fmt;

use std::time::{UNIX_EPOCH, Duration};

use crate::BitcoinAmount;

use crate::BitcoinAddress;
pub use bitcoin::{
     EcdsaSighashType, Network, PrivateKey as BitcoinPrivateKey,
    PublicKey as BitcoinPublicKey, Script,
};
use anyhow::anyhow;

use bitcoin::blockdata::script::Builder;
use ::secp256k1:: SecretKey;

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct BTransaction {
    pub txid: String,
    pub version: i32,
    pub locktime: u32,
    pub vin: Vec<Input>,
    pub vout: Vec<Output>,
    pub size: u64,
    pub weight: u64,
    pub fee: u64,
    pub status: Status,
}


impl BTransaction {
    pub fn overview(btc_wallet: BitcoinWallet, transactions: Vec<BTransaction>, owners_addresses: Vec<String>) -> Result<String, anyhow::Error> {
        
        // We need to know which addresses belong to our wallet
        let our_addresses = btc_wallet.addresses().iter().map(|address| address.public_address_string()).collect::<Vec<String>>();
        
        let mut transactions = transactions;
        // sort the transactions by the block_time
        transactions.sort_by(|a, b| a.status.block_time.cmp(&b.status.block_time));
        if transactions.len() != owners_addresses.len() {
            return Err(anyhow!("transactions and owners_addresses should be of the same length"));
        }
        let mut table = Table::new();
        // Don't list duplicate transactions (same txid) which may have been referenced using different addresses involved in the transaction
        let mut seen_txids = Vec::new();
        // Amount to display is the change in the running balance
        table.add_row(row!["Transaction ID", "Amount (BTC)", "To/From Address", "Status", "Timestamp"]);
        for i in 0..transactions.len() {
            let our_inputs: Vec<Output> = transactions[i].vin.iter().filter(|input| our_addresses.contains(&input.prevout.scriptpubkey_address)).map(|x| x.prevout.clone()).collect();
            let received_outputs: Vec<Output> = transactions[i].vout.iter().filter(|output| our_addresses.contains(&output.scriptpubkey_address)).map(|x| x.clone()).collect();
            let received_amount = BitcoinAmount::new_from_satoshi(received_outputs.iter().fold(0, |acc, output| acc + output.value));
            let sent_amount = BitcoinAmount::new_from_satoshi(our_inputs.iter().fold(0, |acc, output| acc + output.value));
          
            let amount_balance = if received_amount > sent_amount {
                // this is situation when we are receiving money
                (received_amount - sent_amount).btc()
            }
            else {
                // this is the situation where we are sending money
                (sent_amount - received_amount).btc() * -1.0
            };

            let status_string = if transactions[i].status.confirmed {
                "Confirmed".to_string()
            } else {
                "Pending Confirmation".to_string()
            };
            let timestamp = transactions[i].status.timestamp();
            
            if seen_txids.contains(&transactions[i].txid) {
                continue;
            }
            table.add_row(row![transactions[i].txid, amount_balance, owners_addresses[i], status_string, timestamp]);
            seen_txids.push(transactions[i].txid.clone());
        }
        Ok(table.to_string())
    }

    pub fn details(&self, owners_address: String) -> Result<String, anyhow::Error> {
        let mut table_string = String::new();
        let mut table = Table::new();
        table.add_row(row!["Tx ID", self.txid]);
        table_string.push_str(&table.to_string());
        table = Table::new();
        table.add_row(row!["Status", self.status]);
        table_string.push_str(&table.to_string());
        let input_amount = BitcoinAmount::new_from_satoshi(self.vin.iter().fold(0, |acc, input| acc + input.prevout.value)).btc();
        let output_amount = BitcoinAmount::new_from_satoshi(self.vout.iter().fold(0, |acc, output| acc + output.value)).btc();
        let change_amount = BitcoinAmount::new_from_satoshi(self.vout.iter().filter(|output| output.scriptpubkey_address == owners_address).fold(0, |acc, output| acc + output.value)).btc();
        let amount = input_amount - output_amount + change_amount;
        let fee_amount = BitcoinAmount::new_from_satoshi(self.fee).btc();
        table = Table::new();
        table.add_row(row!["Amount: ", amount.to_string()]);
        table.add_row(row!["Fee Amount: ", fee_amount.to_string()]);
        table.add_row(row!["To/From Address: ", owners_address]);
        table.add_row(row!["Version", self.version]);
        table.add_row(row!["Locktime", self.locktime]);
        table.add_row(row!["Size", self.size]);
        table.add_row(row!["Weight", self.weight]);
        table_string.push_str(&table.to_string());
        
        table = Table::new();
        table.add_row(row!["Inputs"]);
        for input in &self.vin {
            table.add_row(row![input.to_string()]);
        }
        table_string.push_str(&table.to_string());
        table = Table::new();
        table.add_row(row!["Outputs"]);
        for output in &self.vout {
            table.add_row(row![output.to_string()]);
        }
        table_string.push_str(&table.to_string());
        Ok(table_string)
    }


     // TODO(AS): add doc here
     pub fn prepare_transaction(
        fee_sat_per_byte: f64,
        utxo_available: &Vec<Utxo>,
        inputs_available_tx_info: &[BTransaction],
        send_amount: &BitcoinAmount,
        receiver_view_wallet: &BitcoinAddress,
        change_addr: Address
    ) -> Result<(BTransaction, Vec<usize>), anyhow::Error> {
        // choose inputs
        let (inputs, fee_amount, chosen_indices)= BitcoinAddress::choose_inputs_and_set_fee(
            utxo_available,
            send_amount,
            inputs_available_tx_info,
            fee_sat_per_byte,
        )?;
        let inputs_amount = BitcoinAmount {
            satoshi: inputs.iter().map(|x| x.prevout.value).sum(),
        };
        if inputs_amount < (*send_amount + fee_amount) {
            return Err(anyhow!("Insufficient funds to send amount and cover fees"));
        }

      
        let change_amount = inputs_amount - *send_amount - fee_amount;

        // Create two outputs, one for the send amount and another for the change amount
        // Hardcoding p2wpkh SegWit transaction option
        // TODO(#83) right away need to add the scriptpubkey info
        let mut outputs: Vec<Output> = Vec::new();
        let mut output_send = Output {
            ..Default::default()
        };
        output_send.value = send_amount.satoshi();
        output_send.set_scriptpubkey_info(receiver_view_wallet.address_info())?;
        outputs.push(output_send);
        let mut output_change = Output {
            ..Default::default()
        };
        output_change.value = change_amount.satoshi();
        output_change.set_scriptpubkey_info(change_addr)?;
        outputs.push(output_change);

        let mut transaction = BTransaction {
            ..Default::default()
        };
        transaction.version = 1;
        transaction.locktime = 0;
        transaction.vin = inputs;
        transaction.vout = outputs.clone();
        transaction.fee = fee_amount.satoshi();

        Ok((transaction, chosen_indices))    
    }


    pub fn sign_tx(&self, keys_per_input: Vec<(BitcoinPrivateKey, BitcoinPublicKey)>) -> Result<Self, anyhow::Error> {
        let mut inputs = self.vin.clone();
        // Signing and unlocking the inputs
        for (i, input) in inputs.iter_mut().enumerate() {
            // hardcoded default to SIGHASH_ALL
            let sighash_type = EcdsaSighashType::All;
            let transaction_hash_for_input_with_sighash = self
                .transaction_hash_for_signing_segwit_input_index(i, sighash_type.to_u32())?;
            let private_key = &keys_per_input[i].0;
            let public_key= &keys_per_input[i].1;
            let secret_key = SecretKey::from_slice(private_key.to_bytes().as_slice())
                .expect("32 bytes, within curve order");
            let sig_with_hashtype = BitcoinAddress::signature_sighashall_for_trasaction_hash(
                transaction_hash_for_input_with_sighash.to_string(),
                secret_key,
            )?;

            // handle the different types of inputs based on previous locking script
            let prevout_lockingscript_type = &input.prevout.scriptpubkey_type;
            match prevout_lockingscript_type.as_str() {
                "p2pkh" => {
                    let script_sig = Builder::new()
                        .push_slice(&hex::decode(sig_with_hashtype)?)
                        .push_key(public_key)
                        .into_script();
                    input.scriptsig_asm = script_sig.asm();
                    input.scriptsig = hex::encode(script_sig.as_bytes());
                }
                "p2sh" => {
                    // TODO(#83) need to handle redeem scripts
                    return Err(anyhow!("Not currently handling P2SH"));
                }
                "v0_p2wsh" => {
                    // TODO(#83) need to handle redeem scripts
                    return Err(anyhow!("Not currently handling v0_p2wsh"));
                }
                "v0_p2wpkh" => {
                    // Need to specify witness data to unlock
                    input.witness = vec![sig_with_hashtype, hex::encode(public_key.to_bytes())];
                }
                _ => {
                    return Err(anyhow!(
                        "Unidentified locking script type from previous output"
                    ))
                }
            }
        }
        let mut signed_tx = self.clone();
        signed_tx.vin = inputs;
        Ok(signed_tx)
    }
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Output {
    pub scriptpubkey: String,
    pub scriptpubkey_asm: String,
    pub scriptpubkey_type: String,
    pub scriptpubkey_address: String,
    pub pubkeyhash: String,
    pub value: u64,
}

impl fmt::Display for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut table = Table::new();
        if !self.scriptpubkey.is_empty() {
            table.add_row(row!["ScriptPubKey", self.scriptpubkey]);
        }
        if !self.scriptpubkey_asm.is_empty() {
            table.add_row(row!["ScriptPubKey ASM", self.scriptpubkey_asm]);
        }
        if !self.scriptpubkey_type.is_empty() {
            table.add_row(row!["ScriptPubKey Type", self.scriptpubkey_type]);
        }
        if !self.scriptpubkey_address.is_empty() {
            table.add_row(row!["ScriptPubKey Address", self.scriptpubkey_address]);
        }
        if !self.pubkeyhash.is_empty() {
            table.add_row(row!["PubKeyHash", self.pubkeyhash]);
        }
        table.add_row(row!["Value (BTC)", BitcoinAmount::new_from_satoshi(self.value).btc()]);
        write!(f, "{}", table)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Input {
    pub txid: String,
    pub vout: u32,
    pub prevout: Output,
    pub scriptsig: String,
    pub scriptsig_asm: String,
    pub witness: Vec<String>,
    pub is_coinbase: bool,
    pub sequence: u32,
    pub inner_redeemscript_asm: String,
}

impl fmt::Display for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut table = Table::new();
        table.add_row(row!["Input Tx ID", self.txid]);
        table.add_row(row!["Amount (BTC)", BitcoinAmount::new_from_satoshi(self.vout as u64).btc()]);
        if !self.scriptsig.is_empty() {
            table.add_row(row!["ScriptSig", self.scriptsig]);
        }
        if !self.scriptsig_asm.is_empty() {
            table.add_row(row!["ScriptSig ASM", self.scriptsig_asm]);
        }
        if !self.witness.is_empty() {
            table.add_row(row!["Witness", self.witness.join(" ")]);
        }
        if !self.inner_redeemscript_asm.is_empty() {
            table.add_row(row!["Inner Redeemscript ASM", self.inner_redeemscript_asm]);
        }
        table.add_row(row!["Is Coinbase", self.is_coinbase]);
        table.add_row(row!["Sequence", self.sequence]);
        write!(f, "{}", table)
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Status {
    pub confirmed: bool,
    pub block_height: u32,
    pub block_hash: String,
    pub block_time: u32,
}

impl Status {

    pub fn timestamp(&self) -> String {
        if self.confirmed {
        // Creates a new SystemTime from the specified number of whole seconds
        let d = UNIX_EPOCH + Duration::from_secs(self.block_time.into());
        // Create DateTime from SystemTime
        let datetime = DateTime::<Utc>::from(d);
        // Formats the combined date and time with the specified format string.
        datetime.format("%Y-%m-%d %H:%M:%S").to_string()
        }
        else {
            "".to_string()
        }
    }
}
impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
       let mut table = Table::new();
       table.add_row(row!["Confirmed: ", self.confirmed]);
         table.add_row(row!["Block Height: ", self.block_height]);
            table.add_row(row!["Block Hash: ", self.block_hash]);
                table.add_row(row!["Timestamp ", self.timestamp()]);
                write!(f, "{}", table)
    }
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Utxo {
    pub status: Status,
    pub txid: String,
    pub value: u64,
    pub vout: u32,
}

pub enum InputType {
    P2pkh,
    P2sh,
    P2wsh,
    P2wpkh,
    P2sh2Wpkh,
    P2sh2Wsh,
}

impl InputType {
    pub fn new(utxo_prevout: &Output) -> Result<Self, anyhow::Error> {
        match utxo_prevout.scriptpubkey_type.as_str() {
            "p2pkh" => Ok(InputType::P2pkh),
            "p2sh" => {
                let scriptpubkey_asm = &utxo_prevout
                    .scriptpubkey_asm
                    .split_whitespace()
                    .map(|x| x.to_string())
                    .collect::<Vec<String>>();
                let op_pushbytes = scriptpubkey_asm.get(1);
                if let Some(op) = op_pushbytes {
                    match op.as_str() {
                        "OP_PUSHBYTES_22" => return Ok(InputType::P2sh2Wpkh),
                        "OP_PUSHBYTES_34" => return Ok(InputType::P2sh2Wsh),
                        _ => return Ok(InputType::P2sh),
                    }
                }
                Ok(InputType::P2sh)
            }
            "v0_p2wsh" => Ok(InputType::P2wsh),
            "v0_p2wpkh" => Ok(InputType::P2wpkh),
            _ => Err(anyhow!("Unknown scriptpubkey_type, not currently handled")),
        }
    }

    pub fn is_segwit(&self) -> bool {
        match self {
            InputType::P2pkh | InputType::P2sh => false,
            InputType::P2sh2Wpkh | InputType::P2sh2Wsh | InputType::P2wsh | InputType::P2wpkh => {
                true
            }
        }
    }
}

impl BTransaction {
    pub fn new_from_value(transaction_info: &Value) -> Result<BTransaction, anyhow::Error> {
        let mut transaction = BTransaction {
            ..Default::default()
        };

        if let Value::Object(object) = transaction_info {
            for obj_item in object {
                if obj_item.0 == "txid" {
                    transaction.txid = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "version" {
                    transaction.version = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "locktime" {
                    transaction.locktime = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "vin" {
                    transaction.vin = Input::new_vector_from_value(obj_item.1)?;
                } else if obj_item.0 == "vout" {
                    transaction.vout = Output::new_vector_from_value(obj_item.1)?;
                } else if obj_item.0 == "size" {
                    transaction.size = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "weight" {
                    transaction.weight = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "fee" {
                    transaction.fee = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "status" {
                    transaction.status = Status::new_from_value(obj_item.1)?;
                }
            }
            return Ok(transaction);
        }
        Err(anyhow!("Transaction info not available"))
    }

    pub fn new_transactions(transactions_info: Value) -> Result<Vec<Self>, anyhow::Error> {
        let mut all_transactions_info: Vec<BTransaction> = Vec::new();
        if transactions_info.is_array() {
            if let Value::Array(vec) = transactions_info {
                for item in vec.iter() {
                    let mut transaction_info = BTransaction {
                        ..Default::default()
                    };
                    if let Value::Object(map) = item {
                        for map_item in map {
                            if map_item.0 == "status" {
                                transaction_info.status = Status::new_from_value(map_item.1)?;
                            } else if map_item.0 == "fee" {
                                transaction_info.fee = serde_json::from_value(map_item.1.clone())?;
                            } else if map_item.0 == "locktime" {
                                transaction_info.locktime =
                                    serde_json::from_value(map_item.1.clone())?;
                            } else if map_item.0 == "size" {
                                transaction_info.size = serde_json::from_value(map_item.1.clone())?;
                            } else if map_item.0 == "txid" {
                                transaction_info.txid = serde_json::from_value(map_item.1.clone())?;
                            } else if map_item.0 == "version" {
                                transaction_info.version =
                                    serde_json::from_value(map_item.1.clone())?;
                            } else if map_item.0 == "vin" {
                                transaction_info.vin = Input::new_vector_from_value(map_item.1)?;
                            } else if map_item.0 == "vout" {
                                transaction_info.vout = Output::new_vector_from_value(map_item.1)?;
                            } else if map_item.0 == "weight" {
                                transaction_info.weight =
                                    serde_json::from_value(map_item.1.clone())?;
                            }
                        }
                        all_transactions_info.push(transaction_info);
                    }
                }
            }
        }
        Ok(all_transactions_info)
    }

    pub fn transaction_hash_for_signing_segwit_input_index(
        &self,
        index: usize,
        sighash_num: u32,
    ) -> Result<String, anyhow::Error> {
        let serialized = self.serialize_for_segwit_input_index_with_sighash(index, sighash_num)?;
        let hash = sha256d::Hash::hash(&hex::decode(serialized)?);
        Ok(hex::encode(hash))
    }

    /// Serializes the transaction for a given input index
    pub fn serialize_for_segwit_input_index_with_sighash(
        &self,
        index: usize,
        sighash_num: u32,
    ) -> Result<String, anyhow::Error> {
        let input = self.vin.get(index).expect("index not present");
        let mut serialization = String::new();

        // nVersion of the transaction (4-byte little endian)
        let version_encoded = self.version.to_le_bytes();
        serialization.push_str(&hex::encode(version_encoded));

        // hashPrevouts, double sha256 hash of the all of the previous outpoints (32
        // byte hash) Ignoring case of ANYONECANPAY
        let mut prevouts_serialized = String::new();
        for input_here in &self.vin {
            let prev_txid = &input_here.txid;
            if prev_txid.len() != 64 {
                return Err(anyhow!(
                    "The references txid in hex format should be 64 characters long"
                ));
            }
            let prev_txid_encoded = Self::hex_reverse_byte_order(prev_txid)?;
            prevouts_serialized.push_str(prev_txid_encoded.as_str());
            let prev_vout: u32 = input_here.vout;
            let prev_vout_encoded = &prev_vout.to_le_bytes();
            prevouts_serialized.push_str(&hex::encode(prev_vout_encoded));
        }

        let hash_prevouts = hex::encode(sha256d::Hash::hash(&hex::decode(prevouts_serialized)?));

        serialization.push_str(hash_prevouts.as_str());

        // hashSequence (using the sequence from each input) (32 byte hash)
        // this is hardcoded right now ignoring case of sighash ANYONECANPAY, SINGLE,
        // NONE
        let mut sequence_serialized = String::new();
        for input_here in &self.vin {
            let sequence_here = input_here.sequence.to_le_bytes();
            sequence_serialized.push_str(hex::encode(sequence_here).as_str());
        }
        let hash_sequence = hex::encode(sha256d::Hash::hash(&hex::decode(sequence_serialized)?));

        serialization.push_str(hash_sequence.as_str());

        // outpoint (32-byte hash + 4-byte little endian)
        let prev_txid = &input.txid;
        if prev_txid.len() != 64 {
            return Err(anyhow!(
                "The references txid in hex format should be 64 characters long"
            ));
        }
        let prev_txid_encoded = Self::hex_reverse_byte_order(prev_txid)?;
        serialization.push_str(prev_txid_encoded.as_str());
        let prev_vout: u32 = input.vout;
        let prev_vout_encoded = &prev_vout.to_le_bytes();
        serialization.push_str(&hex::encode(prev_vout_encoded));

        // scriptCode of the input, hardcoded to p2wpkh
        let pubkeyhash = input.prevout.pubkeyhash.as_str();

        let script_code = "1976a914".to_string() + pubkeyhash + "88ac";
        serialization.push_str(script_code.as_str());

        // value of output spent by this input (8 byte little endian)
        serialization.push_str(&hex::encode(input.prevout.value.to_le_bytes()));

        // nSequence of the input (4 byte little endian)
        serialization.push_str(&hex::encode(input.sequence.to_le_bytes()));

        // hashOutputs (32 byte hash) hardcoding for sighash ALL
        let mut outputs_serialization = String::new();
        for output in &self.vout {
            let value: u64 = output.value;
            let value_encoded = value.to_le_bytes();
            outputs_serialization.push_str(&hex::encode(value_encoded));
            let len_scriptpubkey = output.scriptpubkey.len();
            if len_scriptpubkey % 2 != 0 {
                return Err(anyhow!("Length of scriptpubkey should be a multiple of 2"));
            }
            let len_scriptpubkey_encoded =
                Self::variable_length_integer_encoding(len_scriptpubkey / 2)?;
            outputs_serialization.push_str(&hex::encode(len_scriptpubkey_encoded));
            // scriptpubkey is already encoded for the serialization
            outputs_serialization.push_str(output.scriptpubkey.as_str());
        }
        let hash_outputs = hex::encode(sha256d::Hash::hash(&hex::decode(outputs_serialization)?));
        serialization.push_str(hash_outputs.as_str());
        // Lock Time
        serialization.push_str(&hex::encode(self.locktime.to_le_bytes()));
        // Sighash
        serialization.push_str(&hex::encode(sighash_num.to_le_bytes()));

        Ok(serialization)
    }

    /// Serializes the transaction data (makes a hex string) considering the
    /// data from all of the fields
    pub fn serialize(transaction: &Self) -> Result<String, anyhow::Error> {
        let mut serialization = String::new();
        // version
        let version_encoded = transaction.version.to_le_bytes();
        serialization.push_str(&hex::encode(version_encoded));

        // Handling the segwit marker and flag
        let mut segwit_transaction = false;
        for input in transaction.vin.iter() {
            if !input.witness.is_empty() {
                segwit_transaction = true;
            }
        }

        if segwit_transaction {
            let marker_encoded = "00";
            serialization.push_str(marker_encoded);
            let flag_encoded = "01";
            serialization.push_str(flag_encoded);
        }

        // Inputs
        let num_inputs = transaction.vin.len();
        let num_inputs_encoded = Self::variable_length_integer_encoding(num_inputs)?;
        serialization.push_str(&hex::encode(num_inputs_encoded));
        for input in &transaction.vin {
            let prev_txid = &input.txid;
            if prev_txid.len() != 64 {
                return Err(anyhow!(
                    "The references txid in hex format should be 64 characters long"
                ));
            }
            let prev_txid_encoded = Self::hex_reverse_byte_order(prev_txid)?;
            serialization.push_str(prev_txid_encoded.as_str());
            let prev_vout: u32 = input.vout;
            let prev_vout_encoded = &prev_vout.to_le_bytes();
            serialization.push_str(&hex::encode(prev_vout_encoded));
            let len_signature_script = input.scriptsig.len();
            if len_signature_script % 2 != 0 {
                return Err(anyhow!("Length of script_sig should be a multiple of 2"));
            }
            let len_signature_script_encoded =
                Self::variable_length_integer_encoding(len_signature_script / 2)?;
            serialization.push_str(&hex::encode(len_signature_script_encoded));
            // script_sig is already encoded for the serialization
            serialization.push_str(&input.scriptsig);
            // sequence
            serialization.push_str(&hex::encode(input.sequence.to_le_bytes()));
        }

        // Outputs
        let num_outputs = transaction.vout.len();
        let num_outputs_encoded = Self::variable_length_integer_encoding(num_outputs)?;
        serialization.push_str(&hex::encode(num_outputs_encoded));
        for output in &transaction.vout {
            let value: u64 = output.value;
            let value_encoded = value.to_le_bytes();
            serialization.push_str(&hex::encode(value_encoded));
            let len_scriptpubkey = output.scriptpubkey.len();
            if len_scriptpubkey % 2 != 0 {
                println!(
                    "len_scriptpubkey: {}, {}",
                    len_scriptpubkey, output.scriptpubkey
                );
                return Err(anyhow!("Length of scriptpubkey should be a multiple of 2"));
            }
            let len_scriptpubkey_encoded =
                Self::variable_length_integer_encoding(len_scriptpubkey / 2)?;
            serialization.push_str(&hex::encode(len_scriptpubkey_encoded));
            // scriptpubkey is already encoded for the serialization
            serialization.push_str(output.scriptpubkey.as_str());
        }

        // Witness data
        if segwit_transaction {
            let mut witness_counts: Vec<usize> = Vec::new();
            let mut witness_lens: Vec<u8> = Vec::new();
            let mut witness_data: Vec<String> = Vec::new();

            for (i, input) in transaction.vin.iter().enumerate() {
                witness_counts.push(0);
                for data in &input.witness {
                    witness_counts[i] += 1;
                    if data.len() % 2 != 0 {
                        return Err(anyhow!(
                            "Witness data length in hex should be a multiple of 2"
                        ));
                    }
                    witness_lens.push((data.len() / 2).try_into()?);
                    witness_data.push(data.to_string());
                }
            }
            let mut witness_counter = 0;
            for witness_count in witness_counts {
                serialization.push_str(&hex::encode(Self::variable_length_integer_encoding(
                    witness_count,
                )?));
                for _j in 0..witness_count {
                    serialization
                        .push_str(&hex::encode(witness_lens[witness_counter].to_le_bytes()));
                    serialization.push_str(witness_data[witness_counter].as_str());
                    witness_counter += 1;
                }
            }
        }

        // Lock Time
        serialization.push_str(&hex::encode(transaction.locktime.to_le_bytes()));
        Ok(serialization)
    }

    /// Displays the transaction id in the form used in the blockchain which is
    /// reverse byte of txid()
    pub fn txid_blockchain(&self) -> Result<String, anyhow::Error> {
        let txid = self.txid()?;
        Self::hex_reverse_byte_order(&txid)
    }

    /// Hashes the transaction without including the segwit data
    pub fn txid(&self) -> Result<String, anyhow::Error> {
        let mut transaction = self.clone();
        for input in &mut transaction.vin {
            input.witness = Vec::new();
        }
        let serialization = Self::serialize(&transaction)?;
        let txid = sha256d::Hash::hash(&hex::decode(serialization)?);
        Ok(hex::encode(txid))
    }

    /// Hashes the transaction including all data (including the segwit witness
    /// data)
    pub fn wtxid(&self) -> Result<String, anyhow::Error> {
        let transaction = self.clone();
        let serialization = Self::serialize(&transaction)?;
        let txid = sha256d::Hash::hash(&hex::decode(serialization)?);
        Ok(hex::encode(txid))
    }

    /// Returns the "normalized txid" - sha256 double hash of the serialized
    /// transaction data without including any inputs unlocking data
    /// (witness data and signature, public key data is not included)
    pub fn ntxid(&self) -> Result<String, anyhow::Error> {
        let mut transaction = self.clone();
        for input in &mut transaction.vin {
            input.witness = Vec::new();
            input.scriptsig = String::new();
            input.scriptsig_asm = String::new();
        }
        let serialization = Self::serialize(&transaction)?;
        let ntxid = sha256d::Hash::hash(&hex::decode(serialization)?);
        Ok(hex::encode(ntxid))
    }

    pub fn hex_reverse_byte_order(hex_string: &String) -> Result<String, anyhow::Error> {
        let len = hex_string.len();
        if len % 2 != 0 {
            return Err(anyhow!(
                "The hex string should have a length that is a multiple of 2"
            ));
        }
        let mut encoded = String::new();
        for i in 0..len / 2 {
            let reverse_ind = len - i * 2 - 2;
            encoded.push_str(&hex_string[reverse_ind..reverse_ind + 2]);
        }
        Ok(encoded)
    }

    pub fn variable_length_integer_encoding(num: usize) -> Result<Vec<u8>, anyhow::Error> {
        if num < 0xFD {
            Ok(vec![num as u8])
        } else if num <= 0xFFFF {
            let num_as_bytes = (num as u16).to_le_bytes().to_vec();
            Ok([vec![0xFD], num_as_bytes].concat())
        } else if num <= 0xFFFFFFFF {
            let num_as_bytes = (num as u32).to_le_bytes().to_vec();
            Ok([vec![0xFE], num_as_bytes].concat())
        } else {
            let num_as_bytes = (num as u64).to_le_bytes().to_vec();
            Ok([vec![0xFF], num_as_bytes].concat())
        }
    }
}

impl Default for Input {
    fn default() -> Self {
        Self {
            txid: String::new(),
            vout: 0,
            prevout: Output {
                ..Default::default()
            },
            scriptsig: String::new(),
            scriptsig_asm: String::new(),
            witness: Vec::new(),
            is_coinbase: false,
            sequence: 0xFFFFFFFF,
            inner_redeemscript_asm: String::new(),
        }
    }
}

impl Input {
    pub fn new_from_value(input_info: &Value) -> Result<Input, anyhow::Error> {
        let mut input = Input {
            ..Default::default()
        };
        if let Value::Object(object) = input_info {
            for obj_item in object {
                if obj_item.0 == "txid" {
                    input.txid = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "vout" {
                    input.vout = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "prevout" {
                    input.prevout = Output::new_from_value(obj_item.1)?;
                } else if obj_item.0 == "scriptsig" {
                    input.scriptsig = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "scriptsig_asm" {
                    input.scriptsig_asm = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "witness" {
                    input.witness = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "is_coinbase" {
                    input.is_coinbase = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "sequence" {
                    input.sequence = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "inner_redeemscript_asm" {
                    input.inner_redeemscript_asm = serde_json::from_value(obj_item.1.clone())?;
                }
            }
            Ok(input)
        } else {
            Err(anyhow!("Input info is not there"))
        }
    }

    pub fn new_vector_from_value(vin_info: &Value) -> Result<Vec<Input>, anyhow::Error> {
        let mut vinputs: Vec<Input> = Vec::new();
        if vin_info.is_array() {
            if let Value::Array(vec) = vin_info {
                for item in vec.iter() {
                    let input_info: Input = Input::new_from_value(item)?;
                    vinputs.push(input_info);
                }
            }
            Ok(vinputs)
        } else {
            Err(anyhow!("Info not there for vector of Input"))
        }
    }
}

impl Output {
    pub fn new_from_value(output_info: &Value) -> Result<Output, anyhow::Error> {
        let mut output = Output {
            ..Default::default()
        };
        if let Value::Object(object) = output_info {
            for obj_item in object {
                if obj_item.0 == "scriptpubkey" {
                    output.scriptpubkey = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "scriptpubkey_asm" {
                    output.scriptpubkey_asm = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "scriptpubkey_type" {
                    output.scriptpubkey_type = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "scriptpubkey_address" {
                    output.scriptpubkey_address = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "value" {
                    output.value = serde_json::from_value(obj_item.1.clone())?;
                }
            }
            Ok(output)
        } else {
            Err(anyhow!("Info not availabe for Output"))
        }
    }

    pub fn new_vector_from_value(vout_info: &Value) -> Result<Vec<Output>, anyhow::Error> {
        let mut voutputs: Vec<Output> = Vec::new();
        if vout_info.is_array() {
            if let Value::Array(vec) = vout_info {
                for item in vec.iter() {
                    let output_info: Output = Output::new_from_value(item)?;
                    voutputs.push(output_info);
                }
            }
            Ok(voutputs)
        } else {
            Err(anyhow!("Info not there for vector of Output"))
        }
    }

    pub fn set_scriptpubkey_info(&mut self, address_info: Address) -> Result<(), anyhow::Error> {
        self.scriptpubkey_address = address_info.to_string();
        let address_type = address_info.address_type().expect("address type missing");
        match address_type {
            AddressType::P2pkh => self.scriptpubkey_type = "p2pkh".to_string(),
            AddressType::P2sh => self.scriptpubkey_type = "p2sh".to_string(),
            AddressType::P2wpkh => self.scriptpubkey_type = "v0_p2wpkh".to_string(),
            AddressType::P2wsh => self.scriptpubkey_type = "v0_p2wsh".to_string(),
            _ => {
                return Err(anyhow!(
                    "Currently not implemented setting scriptpubkey for this address type"
                ))
            }
        }
        let script_pubkey = address_info.script_pubkey();
        self.scriptpubkey_asm = script_pubkey.asm();
        self.scriptpubkey = hex::encode(script_pubkey.as_bytes());
        Ok(())
    }
}

impl Status {
    pub fn new_from_value(status_info: &Value) -> Result<Status, anyhow::Error> {
        let mut status = Status {
            ..Default::default()
        };
        if let Value::Object(object) = status_info {
            for obj_item in object {
                if obj_item.0 == "confirmed" {
                    status.confirmed = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "block_height" {
                    status.block_height = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "block_hash" {
                    status.block_hash = serde_json::from_value(obj_item.1.clone())?;
                } else if obj_item.0 == "block_time" {
                    status.block_time = serde_json::from_value(obj_item.1.clone())?;
                }
            }
            Ok(status)
        } else {
            Err(anyhow!("status info not available"))
        }
    }
}

impl Utxo {
    pub fn new_utxos(utxo_info: Value) -> Result<Vec<Utxo>, anyhow::Error> {
        let mut all_utxo_info: Vec<Utxo> = Vec::new();
        if utxo_info.is_array() {
            if let Value::Array(vec) = utxo_info {
                for item in vec.iter() {
                    let mut utxo: Utxo = Utxo {
                        ..Default::default()
                    };
                    if let Value::Object(map) = item {
                        for map_item in map {
                            if map_item.0 == "status" {
                                utxo.status = Status::new_from_value(map_item.1)?;
                            } else if map_item.0 == "txid" {
                                utxo.txid = serde_json::from_value(map_item.1.clone())?;
                            } else if map_item.0 == "value" {
                                utxo.value = serde_json::from_value(map_item.1.clone())?;
                            } else if map_item.0 == "vout" {
                                utxo.vout = serde_json::from_value(map_item.1.clone())?;
                            }
                        }
                    }
                    all_utxo_info.push(utxo);
                }
            }
            Ok(all_utxo_info)
        } else {
            Err(anyhow!("utxo_info was not array value"))
        }
    }
}
#[derive(Clone, Default, Debug)]
pub struct Blockstream {
    pub client: reqwest::Client,
    pub url: String,
}

pub const BLOCKSTREAM_TESTNET_URL: &str = "https://blockstream.info/testnet/api";
pub const BLOCKSTREAM_URL: &str = "https://blockstream.info/api";

#[async_trait]
impl BlockchainConnector for Blockstream {
    fn new(url: &str) -> Result<Self, anyhow::Error> {
        Ok(Self {
            client: reqwest::Client::new(),
            url: url.to_string(),
        })
    }

    async fn check_if_past_transactions_exist(
        &self,
        public_address: &str,
    ) -> Result<bool, anyhow::Error> {
        let transactions = self.transactions(public_address).await?;
        if transactions.is_empty() {
            // println!("No past transactions exist at address: {}", public_address);
            return Ok(false);
        } else {
            // println!("Past transactions exist at address: {}", public_address);
            return Ok(true);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone, Default, Debug, Deserialize, Serialize)]
pub struct FeeEstimates(pub serde_json::Map<String, Value>);


impl fmt::Display for FeeEstimates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut table = Table::new();
        writeln!(f, "Fee Estimates")?;
        table.add_row(row!["Confirmation Target (Blocks)", "Fee (sat/vB)"]);
        let mut keys = self.0.iter().map(|(a, _b)| a.parse::<u32>().expect("expecting that key should be able to be parsed as u32")).collect::<Vec<_>>();
        keys.sort();
        for key in keys {
           table.add_row(row![key, self.0[&key.to_string()]]);
        }
        write!(f, "{}", table)?;
        Ok(())
    }
}

impl Blockstream {
    // fetch the block height
    pub fn block_count(&self) -> Result<u64, anyhow::Error> {
        let body = reqwest::blocking::get(format!("{}/blocks/tip/height", self.url))
            .expect("Error getting block count")
            .text()?;
        println!("body = {:?}", body);
        let block_count: u64 = body.parse()?;
        Ok(block_count)
    }

    // fetch fee estimates from blockstream
    pub async fn fee_estimates(&self) -> Result<FeeEstimates, anyhow::Error> {
        let body = reqwest::get(format!("{}/fee-estimates", self.url))
            .await?
            .text()
            .await?;
        let fee_estimates: FeeEstimates = serde_json::from_str(&body)?;
        Ok(fee_estimates)
    }

    // fetch transactions from blockstream
    pub async fn transactions(&self, address: &str) -> Result<Vec<BTransaction>, anyhow::Error> {
        let body = reqwest::get(format!("{}/address/{}/txs", self.url, address))
            .await?
            .text()
            .await?;
        let transactions: Value = serde_json::from_str(&body)?;
        BTransaction::new_transactions(transactions)
    }

    // fetch mempool transactions from blockstream
    pub fn mempool_transactions(&self, address: &str) -> Result<Value, anyhow::Error> {
        let body = reqwest::blocking::get(format!("{}/address/{}/txs/mempool", self.url, address))
            .expect("Error getting transactions")
            .text();
        println!("body = {:?}", body);
        let transactions = json!(&body?);
        Ok(transactions)
    }

    /// Fetch UTXOs from blockstream
    pub async fn utxo(&self, address: &str) -> Result<Vec<Utxo>, anyhow::Error> {
        let body = reqwest::get(format!("{}/address/{}/utxo", self.url, address))
            .await?
            .text()
            .await?;

        let utxo: Value = serde_json::from_str(&body)?;
        Utxo::new_utxos(utxo)
    }

    pub async fn get_raw_transaction_hex(&self, txid: &str) -> Result<String, anyhow::Error> {
        let body = reqwest::get(format!("{}/tx/{}/raw", self.url, txid))
            .await?
            .text()
            .await?;
        let raw_transaction_hex = json!(&body);
        Ok(raw_transaction_hex.to_string())
    }

    /// Fetch transaction info
    pub async fn transaction(&self, txid: &str) -> Result<BTransaction, anyhow::Error> {
        let body = reqwest::get(format!("{}/tx/{}", self.url, txid))
            .await?
            .text()
            .await?;
        let _data = r#"{
        "txid":"6249b166d78529e435628245034df9e4c81d9b34b4d12c5600527c96b6e0d8ce",
        "version":1,
        "locktime":0,
        "vin":[
          {
            "txid":"4894c96e044bd6c278f927a220c42048602e4d8bfa888f5c35610b1c4643140d",
            "vout":1,
            "prevout":{
              "scriptpubkey":"a914f7861160df5cce001291293dfba24923816fc7e987",
              "scriptpubkey_asm":"OP_HASH160 OP_PUSHBYTES_20 f7861160df5cce001291293dfba24923816fc7e9 OP_EQUAL",
              "scriptpubkey_type":"p2sh",
              "scriptpubkey_address":"3QFoS8FPLCiVzzra4TPqVCq5ntpswP9Ey3",
              "value":48713312
            },
            "scriptsig":"160014630cf4b24dbd691fef2bb3fa50605484632f611e",
            "scriptsig_asm":"OP_PUSHBYTES_22 0014630cf4b24dbd691fef2bb3fa50605484632f611e",
            "witness":[
              "304402201e23c13611331720f5dfe2455b2d3c3b259d84cadc5e3de6e792a750978efeb8022006d3acad3c1c5b7e6227c80b71fee635f9303fd164b378c98a9fb3063105ff9201",
              "025e7a3239de2b1dbde8d8ff5c0c620ac47bfd32e761f509c13424fe8481dbb98e"
            ],
            "is_coinbase":false,
            "sequence":4294967295,
            "inner_redeemscript_asm":"OP_0 OP_PUSHBYTES_20 630cf4b24dbd691fef2bb3fa50605484632f611e"
          }
        ],
        "vout":[
          {
            "scriptpubkey":"a914b3efe280e64077202c171cc3fefb4bb02adc7d0687",
            "scriptpubkey_asm":"OP_HASH160 OP_PUSHBYTES_20 b3efe280e64077202c171cc3fefb4bb02adc7d06 OP_EQUAL",
            "scriptpubkey_type":"p2sh",
            "scriptpubkey_address":"3J6SFNJSHq9k6k2Cwzdy6RMC1z3ubR1ot1",
            "value":15632000
          },
          {
            "scriptpubkey":"a91445a3f3cc49da0b67c969771b0b8ef76c45aaff2787",
            "scriptpubkey_asm":"OP_HASH160 OP_PUSHBYTES_20 45a3f3cc49da0b67c969771b0b8ef76c45aaff27 OP_EQUAL",
            "scriptpubkey_type":"p2sh",
            "scriptpubkey_address":"383ExPThK2M5yZEtHXU1YcqehVBDxHKuWJ",
            "value":33065280
          }
        ],
        "size":247,
        "weight":661,
        "fee":16032,
        "status":{
          "confirmed":true,
          "block_height":663393,
          "block_hash":"0000000000000000000efbc1d707a0b95bc281c908ecf1f149d2d93ca8d6a175",
          "block_time":1609181106
        }
      }"#;
        let transaction_info: Value = serde_json::from_str(&body)?;
        let transaction: BTransaction = BTransaction::new_from_value(&transaction_info)?;
        Ok(transaction)
    }

    /// Broadcast a raw transaction to the network
    pub async fn post_a_transaction(
        &self,
        raw_transaction_hex: &'static str,
    ) -> Result<String, anyhow::Error> {
        let trans_resp = self
            .client
            .post(format!("{}/tx", self.url))
            .body(raw_transaction_hex)
            .send()
            .await
            .expect("Transaction failed to be posted");

        let trans_status = trans_resp.status();
        let trans_content = trans_resp.text().await?;
        if !trans_status.is_client_error() && !trans_status.is_server_error() {
            Ok(trans_content)
        } else {
            println!(
                "trans_status.is_client_error(): {}",
                trans_status.is_client_error()
            );
            println!(
                "trans_status.is_server_error(): {}",
                trans_status.is_server_error()
            );
            println!("trans_content: {}", trans_content);
            Err(anyhow!("Error in broadcasting the transaction"))
        }
    }
}

#[cfg(test)]
mod tests {
    use mockito::mock;

    use super::*;

    #[test]
    fn test_block_count() {
        let _m = mock("GET", "/blocks/tip/height")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("773876")
            .create();

        let _url: &String = &mockito::server_url();
        let bs = Blockstream::new(&mockito::server_url()).unwrap();
        let check = bs.block_count().unwrap();
        assert_eq!(773876, check);
    }
}
