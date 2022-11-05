use aes::Aes256;
use aes::cipher::{
    BlockCipher, BlockEncrypt, BlockDecrypt, KeyInit,
    generic_array::GenericArray,
};
use scrypt::Scrypt;
use scrypt::password_hash::{SaltString, PasswordHasher};

// Generate 32 bytes (256 bit) secret
pub fn kdf(password: &String) -> Result<[u8; 32], Box<dyn std::error::Error>> {
	// Use fixed salt
	let salt = SaltString::b64_encode(&[0, 0, 0, 0])?;
	// Hash password to PHC string ($scrypt$...)
	let secret = Scrypt.hash_password(password.as_bytes(), &salt)?;
	let binding = secret.hash.unwrap();
 	let bytes = binding.as_bytes();
	let mut result: [u8; 32] = [0; 32];
	for i in 0..32 {
		result[i] = bytes[i].clone();
	}
	Ok(result)
}

const AES_BLOCK_SIZE: usize = 16;

pub fn encrypt(key: [u8; 32], data: Vec<u8>) -> Vec<u8> {
	let key = GenericArray::from(key);
	// Initialize cipher
	let cipher = Aes256::new(&key);
	// Encrypt
	let mut i = 0;
	let mut result: Vec<u8> = Vec::new();
	loop {
		let mut buffer = [0u8;AES_BLOCK_SIZE];
		for j in 0..AES_BLOCK_SIZE {
			if (i + j) >= data.len() {
				break;
			}
			buffer[j] = data[i + j];
		}
		let mut block = GenericArray::from(buffer);
		cipher.encrypt_block(&mut block);
		i += AES_BLOCK_SIZE;
		result.append(&mut block.to_vec());
		if i >= data.len() {
			break;
		}
	}
	result
}

pub fn decrypt(key: [u8; 32], data: Vec<u8>) -> Vec<u8> {
	let key = GenericArray::from(key);
	// Initialize cipher
	let cipher = Aes256::new(&key);
	// Encrypt
	let mut i = 0;
	let mut result: Vec<u8> = Vec::new();
	loop {
		let mut buffer = [0u8;AES_BLOCK_SIZE];
		for j in 0..AES_BLOCK_SIZE {
			if (i + j) >= data.len() {
				break;
			}
			buffer[j] = data[i + j];
		}
		let mut block = GenericArray::from(buffer);
		cipher.decrypt_block(&mut block);
		i += AES_BLOCK_SIZE;
		if i >= data.len() {
			let r = &mut block.to_vec();
			while r[r.len() -1] == 0 {
				r.pop();
			}
			result.append(r);
			break;
		} else {
			result.append(&mut block.to_vec());
		}
	}
	result
}

#[cfg(test)]
mod tests {
    use crate::crypto::decrypt;

    use super::{kdf, encrypt};

    
	#[test]
	fn test_aes() {
		let pwd = "password".to_string();
		let key = kdf(&pwd).unwrap();
		let data = "cpchaincpchaincpchaincpchaincpchaincpchaincpchaincpchain";
		println!("Data: {:x?}", data.as_bytes());
		let encryted = encrypt(key, data.as_bytes().to_vec());
		println!("Encrypted data: {:x?}", encryted.as_slice());
		let decrypted = decrypt(key, encryted.clone());
		println!("Decrypted data: {:x?}", decrypted.as_slice());
	}
}
