use std::time::SystemTime;
use std::{collections::HashMap, time::Duration};
use std::hash::Hash;

#[derive(Debug, Clone)]
struct WithTimestamp<V> where V: Clone {
	value: V,
	ts: SystemTime
}

impl <V> WithTimestamp<V> where V: Clone {
	fn new(value: V) -> WithTimestamp<V> {
		let ts = SystemTime::now();
		Self {
			value,
			ts
		}
	}
}

#[derive(Debug, Clone)]
pub struct Cache<K, V> where K: Eq + Hash, V: Clone {
	data: HashMap<K, WithTimestamp<V>>,
	ttl: Duration
}

impl<K, V> Cache<K, V> where K: Eq + Hash, V: Clone {
	pub fn new(ttl: Duration) -> Self {
		Self {
			ttl,
			data: HashMap::new()
		}
	}
	pub fn len(&self) -> usize {
		self.data.len()
	}
	pub fn contains_key(&self, key: &K) -> bool {
		self.data.contains_key(key)
	}
	pub fn get(&self, key: &K) -> Option<V> {
		if !self.contains_key(key) {
			return None
		}
		return Some(self.data.get(key).unwrap().value.clone())
	}
	pub fn put(&mut self, key: K, value: V) {
		self.data.insert(key, WithTimestamp::new(value));
	}
	pub fn remove(&mut self, k: &K) -> Option<V> {
		match self.data.remove(k) {
			Some(value) => Some(value.value),
			None => None,
		}

	}
	// Clear timeout
	pub async fn compact(&mut self) {
		// iterate through all the keys, check is
		let now = SystemTime::now();
		self.data.retain(|_, value| {
			now.duration_since(value.ts).unwrap().lt(&self.ttl)
		});
	}
}
