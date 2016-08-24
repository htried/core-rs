use ::futures::BoxFuture;
use ::hyper::status::StatusCode;

use ::crypto;

quick_error! {
    #[derive(Debug)]
    pub enum TError {
        Shutdown {
            description("shutting down")
            display("shutting down")
        }
        Msg(str: String) {
            description(str)
            display("error: {}", str)
        }
        BadValue(str: String) {
            description(str)
            display("bad value: {}", str)
        }
        MissingField(str: String) {
            description(str)
            display("missing field: {}", str)
        }
        MissingData(str: String) {
            description(str)
            display("missing data: {}", str)
        }
        CryptoError(err: crypto::CryptoError) {
            cause(err)
            description("crypto error")
            display("crypto error: {}", err)
        }
        ApiError(status: StatusCode) {
            description("API error")
            display("api error: {}", status.canonical_reason().unwrap_or("unknown"))
        }
        TryAgain {
            description("try again")
            display("try again")
        }
        NotImplemented {
            description("not implemented")
            display("not implemented")
        }
    }
}

pub type TResult<T> = Result<T, TError>;
pub type TFutureResult<T> = BoxFuture<T, TError>;

/// converts non-TError errors to TError. this is a macro because I am sure this
/// is the "wrong" way to do this and once I know a better way, I can hopefully
/// fix it later
#[macro_export]
macro_rules! toterr {
    ($e:expr) => (TError::Msg(format!("{}", $e)))
}

/// try!-esque wrapper around toterr
#[macro_export]
macro_rules! try_t {
    ($e:expr) => (try!($e.map_err(|e| toterr!(e))))
}

/// Like try_t! but specifically for when we know we're going to get a crypto
/// error.
#[macro_export]
macro_rules! try_c {
    ($e:expr) => (try!($e.map_err(|e| TError::CryptoError(e))))
}

