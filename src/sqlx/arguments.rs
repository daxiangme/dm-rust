use crate::sqlx::error::dec_err;
use crate::sqlx::Dm;
use crate::conn::Value;
use sqlx_core::arguments::Arguments;
use sqlx_core::encode::Encode;
use sqlx_core::error::BoxDynError;
use sqlx_core::types::Type;

/// 一次执行的参数列表。每个参数是一个已经编码好的 [`Value`]。
#[derive(Debug, Default, Clone)]
pub struct DmArguments {
    pub(crate) values: Vec<Value>,
}

/// `Encode` 写入的单值缓冲。生命周期由 `Database::ArgumentBuffer<'q>` 携带，这里不借用参数。
#[derive(Debug, Default)]
pub struct DmArgumentBuffer {
    pub(crate) value: Option<Value>,
}

impl<'q> Arguments<'q> for DmArguments {
    type Database = Dm;

    fn reserve(&mut self, additional: usize, _size: usize) {
        self.values.reserve(additional);
    }

    fn add<T>(&mut self, value: T) -> Result<(), BoxDynError>
    where
        T: 'q + Encode<'q, Self::Database> + Type<Self::Database>,
    {
        let mut buf = DmArgumentBuffer::default();
        if value.encode(&mut buf)?.is_null() {
            self.values.push(Value::Null);
            return Ok(());
        }
        self.values.push(
            buf.value
                .ok_or_else(|| dec_err("encode reported a value but wrote nothing"))?,
        );
        Ok(())
    }

    fn len(&self) -> usize {
        self.values.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlx::Dm;
    use sqlx_core::encode::Encode;

    #[test]
    fn encodes_i32_string_and_null() {
        let mut args = DmArguments::default();
        args.add(1_i32).unwrap();
        args.add("hi").unwrap();
        args.add(None::<i32>).unwrap();
        assert_eq!(args.len(), 3);
        assert_eq!(args.values[0], Value::Int(1));
        assert_eq!(args.values[1], Value::String("hi".into()));
        assert_eq!(args.values[2], Value::Null);
    }

    #[test]
    fn i32_is_not_narrowed() {
        let mut buf = DmArgumentBuffer::default();
        let _ = Encode::<Dm>::encode(7_i32, &mut buf).unwrap();
        assert_eq!(buf.value, Some(Value::Int(7)));
    }
}
