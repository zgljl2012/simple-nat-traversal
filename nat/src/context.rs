use crate::crypto::kdf;


#[derive(Debug, Clone)]
pub struct Context {
	secret: [u8; 32], // Generate by KDF and password
	ssh_mtu: u16
}

impl Context {
	pub fn new(password: String, ssh_mtu: u16) -> Result<Self, Box<dyn std::error::Error>> {
		Ok(Self {
			secret: kdf(&password)?,
			ssh_mtu
		})
	}

	pub fn get_secret(&self) -> [u8; 32]{
		self.secret.clone()
	}

	pub fn get_ssh_mtu(&self) -> usize {
		self.ssh_mtu as usize
	}
}

#[cfg(test)]
mod tests {
    use super::kdf;

	#[test]
	fn test_kdf() -> Result<(), Box<dyn std::error::Error>> {
		let s = kdf(&"password".to_string())?;
		assert!(s.len() == 32);
		Ok(())
	}
}
