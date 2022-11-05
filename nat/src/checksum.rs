// 校验和

/// 校验和生成算法
/// 
/// 1. 设置校验和字段为0
/// 2. 将需要设置校验和的数据以 16 位一组，依次反码求和
/// 3. 最后的结果就是校验和，存入校验和字段
/// 
/// 校验和校验算法
/// 
/// 1. 包括校验和在内，依次以 16 位一组进行反码求和
/// 2. 结果为 0 则正确，否则错误
/// 
/// 本函数只负责计算校验和（即包括生成，也包括验证），设置校验和字段为 0 操作需在调用前执行
#[allow(unused)]
pub fn checksum(bytes: &[u8]) -> u16 {
	let mut bytes = bytes.clone().to_vec();
	// 末尾补 0
	let padding_zero = bytes.len() % 2 != 0;
	if padding_zero {
		bytes.push(0 as u8);
	}
	// iterate additional checksum
	let mut checksum: u16 = 0;

	// calculate checksum
	for (index, value) in bytes.iter().enumerate() {
		if index % 2 == 0 {
			// 组成一个 u16
			let v = ((*value as u16) << 8) + (bytes[index+1] << 0) as u16;
			// 取反、累加
			checksum = (((!v as u32) + (checksum as u32))) as u16;
		}
	}
	checksum
}


#[cfg(test)]
mod tests {
    use crate::utils;

    use super::checksum;

	#[test]
	fn test_checksum() {
		let mut bytes = vec![6, 2, 0, 0, 255, 255, 3, 4, 1];
		// 第三、四字节置 0
		bytes[2] = 0;
		bytes[3] = 0;
		let cs = checksum(bytes.as_slice());
		let cs_bytes = utils::u16_to_be(cs);
		// 设置校验和
		bytes[2] = cs_bytes[0];
		bytes[3] = cs_bytes[1];
		// validate
		let validated = checksum(bytes.as_slice());
		println!("checksum\n: {} {}", cs, validated);
	}
}
