use std::sync::{Arc, RwLock};

use minijinja::{ErrorKind, value::DynObject};

pub(crate) fn new_value_vec() -> DynObject {
    DynObject::new(Arc::new(ValueVec(RwLock::new(vec![]))))
}

impl minijinja::value::Object for ValueVec {
    fn repr(self: &Arc<Self>) -> minijinja::value::ObjectRepr {
        minijinja::value::ObjectRepr::Iterable
    }

    fn call_method(
        self: &Arc<Self>,
        _state: &minijinja::State<'_, '_>,
        method: &str,
        args: &[minijinja::Value],
    ) -> Result<minijinja::Value, minijinja::Error> {
        match method {
            "push" => self.push(args),
            _ => Err(minijinja::Error::new(
                ErrorKind::UnknownMethod,
                format!("Unexpected method {method}"),
            )),
        }
    }

    fn enumerate(self: &Arc<Self>) -> minijinja::value::Enumerator {
        let vals = self
            .0
            .read()
            .expect("Unable to read from ValueVec, RwLock was poisoned")
            .iter()
            .map(|s| s.to_owned())
            .collect::<Vec<_>>();
        minijinja::value::Enumerator::Iter(Box::new(vals.into_iter()))
    }
}

/// List of `minijinja::Value`, a workaround for `minijinja`s mutability limitations.
///
/// Like `minijinja`s own `namespace`, but in array form instead of dict form.
#[derive(Debug)]
struct ValueVec(RwLock<Vec<minijinja::Value>>);

impl ValueVec {
    fn push(
        self: &Arc<Self>,
        args: &[minijinja::Value],
    ) -> Result<minijinja::Value, minijinja::Error> {
        ensure_n_args("push", 1, args)?;
        {
            let mut list = self
                .0
                .try_write()
                .map_err(|e| minijinja::Error::new(ErrorKind::InvalidOperation, e.to_string()))?;
            list.push(args[0].clone());
        }
        Ok(minijinja::Value::UNDEFINED)
    }
}

fn ensure_n_args(
    method: &str,
    n: usize,
    args: &[minijinja::Value],
) -> Result<(), minijinja::Error> {
    let err = |kind| -> Result<(), minijinja::Error> {
        Err(minijinja::Error::new(
            kind,
            format!(
                "{method} | Expected: {n} args, got {} arguments",
                args.len()
            ),
        ))
    };

    match args.len().cmp(&n) {
        std::cmp::Ordering::Less => err(ErrorKind::MissingArgument),
        std::cmp::Ordering::Greater => err(ErrorKind::TooManyArguments),
        std::cmp::Ordering::Equal => Ok(()),
    }
}
