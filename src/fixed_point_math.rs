use ethers::types::{I256, U256};

pub mod math {
    use ethers::types::U256;

    pub const WAD: U256 = make_u256(1_000_000_000_000_000_000u64);

    pub const fn make_u256(x: u64) -> U256 {
        U256([x, 0, 0, 0])
    }
}
pub trait FixedPointMathGen<T> {
    fn mul_div_down(&self, y: T, denominator: T) -> T;
}
pub trait FixedPointMath {
    fn mul_div_up(&self, y: Self, denominator: Self) -> Self;
    fn mul_wad_down(&self, y: Self) -> Self;
    fn mul_wad_up(&self, y: Self) -> Self;
    fn div_wad_down(&self, y: Self) -> Self;
    fn div_wad_up(&self, y: Self) -> Self;
    fn ln_wad(&self) -> I256;
}

#[inline(always)]
fn lt(x: U256, y: U256) -> U256 {
    if x < y {
        U256::from(1u32)
    } else {
        U256::zero()
    }
}

fn log2(x: U256) -> U256 {
    let mut r;
    r = lt(U256::from(0xffffffffffffffffffffffffffffffffu128), x) << 7u32;
    r = r | (lt(U256::from(0xffffffffffffffffu128), x >> r) << 6u32);
    r = r | (lt(U256::from(0xffffffffu128), x >> r) << 5u32);
    r = r | (lt(U256::from(0xffffu128), x >> r) << 4u32);
    r = r | (lt(U256::from(0xffu128), x >> r) << 3u32);
    r = r | (lt(U256::from(0xfu128), x >> r) << 2u32);
    r = r | (lt(U256::from(0x3u128), x >> r) << 1u32);
    r = r | lt(U256::from(0x1u128), x >> r);
    r
}

impl FixedPointMathGen<I256> for U256 {
    fn mul_div_down(&self, y: I256, denominator: I256) -> I256 {
        let z = I256::from_raw(*self) * y;
        z / denominator
    }
}

impl FixedPointMathGen<U256> for U256 {
    fn mul_div_down(&self, y: Self, denominator: Self) -> Self {
        let z = self * y;
        z / denominator
    }
}

impl FixedPointMath for U256 {
    fn ln_wad(&self) -> I256 {
        let mut x = *self;
        let k = I256::from_raw(log2(x)) - I256::from(96u32);
        x <<= (I256::from(159u32) - k).into_raw();
        let x = I256::from_raw(x >> 159u32);

        let mut p = x + I256::from(3273285459638523848632254066296u128);
        p = ((p * x) >> 96) + I256::from(24828157081833163892658089445524u128);
        p = ((p * x) >> 96) + I256::from(43456485725739037958740375743393u128);
        p = ((p * x) >> 96) - I256::from(11111509109440967052023855526967u128);
        p = ((p * x) >> 96) - I256::from(45023709667254063763336534515857u128);
        p = ((p * x) >> 96) - I256::from(14706773417378608786704636184526u128);
        p = p * x - (I256::from(795164235651350426258249787498u128) << 96);

        let mut q = x + I256::from(5573035233440673466300451813936u128);
        q = ((q * x) >> 96) + I256::from(71694874799317883764090561454958u128);
        q = ((q * x) >> 96) + I256::from(283447036172924575727196451306956u128);
        q = ((q * x) >> 96) + I256::from(401686690394027663651624208769553u128);
        q = ((q * x) >> 96) + I256::from(204048457590392012362485061816622u128);
        q = ((q * x) >> 96) + I256::from(31853899698501571402653359427138u128);
        q = ((q * x) >> 96) + I256::from(909429971244387300277376558375u128);

        let mut r = p / q;

        r *= I256::from_raw(
            U256::from_str_radix("1677202110996718588342820967067443963516166", 10).unwrap(),
        );
        r += I256::from_raw(
            U256::from_str_radix(
                "16597577552685614221487285958193947469193820559219878177908093499208371",
                10,
            )
            .unwrap(),
        ) * k;
        r += I256::from_raw(
            U256::from_str_radix(
                "600920179829731861736702779321621459595472258049074101567377883020018308",
                10,
            )
            .unwrap(),
        );

        r >>= 174;

        r
    }

    fn mul_wad_down(&self, y: Self) -> Self {
        self.mul_div_down(y, math::WAD)
    }

    fn mul_wad_up(&self, y: Self) -> Self {
        self.mul_div_up(y, math::WAD)
    }

    fn div_wad_down(&self, y: Self) -> Self {
        self.mul_div_down(math::WAD, y)
    }

    fn div_wad_up(&self, y: Self) -> Self {
        self.mul_div_up(math::WAD, y)
    }

    fn mul_div_up(&self, y: Self, denominator: Self) -> Self {
        let z: I256 = I256::from_raw(self * y);
        let m = if z.is_zero() {
            I256::zero()
        } else {
            I256::from(1u32)
        };
        (m * ((z - I256::from(1u32)) / I256::from_raw(denominator) + I256::from(1u32))).into_raw()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::types::U256;

    #[test]
    fn test_ln_wad() {
        let a = U256::exp10(19);
        assert_eq!(a.ln_wad(), I256::from(2_302_585_092_994_045_683u128));
    }

    #[test]
    fn test_log_2() {
        let a = U256::from(1024u32);
        assert_eq!(super::log2(a), U256::from(10u32));
    }

    #[test]
    fn test_mul_div_up() {
        let a: U256 = U256::from(5u32);
        let r1 = a.mul_div_up(U256::from(2u32), U256::from(6u32));
        let r2 = a.mul_div_up(U256::from(2u32), U256::from(5u32));
        assert_eq!(r1, U256::from(2u32));
        assert_eq!(r2, U256::from(2u32));
        let r5 = U256::mul_div_up(&U256::from(4u32), U256::from(5u32), U256::from(3u32));
        let r6 = U256::mul_div_up(&U256::from(4u32), U256::from(5u32), U256::from(10u32));
        assert_eq!(r5, U256::from(7u32));
        assert_eq!(r6, U256::from(2u32));
    }

    #[test]
    fn test_mul_div_down() {
        let a: U256 = U256::from(5u32);
        let r1 = a.mul_div_down(U256::from(2u32), U256::from(6u32));
        let r2 = a.mul_div_down(U256::from(2u32), U256::from(5u32));
        assert_eq!(r1, U256::from(1u32));
        assert_eq!(r2, U256::from(2u32));
        let r5 = U256::mul_div_down(&U256::from(4u32), U256::from(5u32), U256::from(3u32));
        let r6 = U256::mul_div_down(&U256::from(4u32), U256::from(5u32), U256::from(10u32));
        assert_eq!(r5, U256::from(6u32));
        assert_eq!(r6, U256::from(2u32));
    }
}
