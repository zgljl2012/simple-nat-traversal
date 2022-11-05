use scrypt::{password_hash::{SaltString, PasswordHasher}, Scrypt};


#[derive(Debug, Clone)]
pub struct Context {
	secret: String, // Generate by KDF and password
	ssh_mtu: u16
}

fn kdf(password: &String) -> Result<String, Box<dyn std::error::Error>> {
	// Use fixed salt
	let salt = SaltString::b64_encode(&[0, 0, 0, 0])?;
	// Hash password to PHC string ($scrypt$...)
	Ok(Scrypt.hash_password(password.as_bytes(), &salt)?.to_string())
}

impl Context {
	pub fn new(password: String, ssh_mtu: u16) -> Result<Self, Box<dyn std::error::Error>> {
		Ok(Self {
			secret: kdf(&password)?,
			ssh_mtu
		})
	}

	pub fn get_secret(&self) -> String {
		self.secret.clone()
	}

	pub fn get_ssh_mtu(&self) -> usize {
		self.ssh_mtu as usize
	}
}
