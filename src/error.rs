//! Defines error types.

use std::fmt::Debug;
use std::num::NonZeroI32;
use std::sync::Arc;

use crate::bindings::BluetoothStatusCodes;

/// Internal error type, not compatible with `bluest::Error`.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum NativeError {
    GattError(AttError),
    BluetoothStatusCode(BluetoothStatusCode),
    JavaError(Arc<jni::errors::Error>),
    JavaCallReturnedFalse,
}

impl std::error::Error for NativeError {}

impl std::fmt::Display for NativeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GattError(att_error) => write!(f, "GATT error: {att_error}"),
            Self::BluetoothStatusCode(st) => write!(f, "{st}"),
            Self::JavaError(err) => write!(f, "Java/JNI error: {err:?}"),
            Self::JavaCallReturnedFalse => write!(f, "Java call unexpectedly returned false"),
        }
    }
}

impl From<jni::errors::Error> for NativeError {
    fn from(e: jni::errors::Error) -> Self {
        Self::JavaError(Arc::new(e))
    }
}

impl From<jni::errors::Error> for crate::Error {
    fn from(e: jni::errors::Error) -> Self {
        let native_err = NativeError::from(e);
        let message = format!("{native_err}");
        Self {
            kind: ErrorKind::Internal,
            source: Some(native_err),
            message,
        }
    }
}

impl From<AttError> for NativeError {
    fn from(att_error: AttError) -> Self {
        Self::GattError(att_error)
    }
}

impl From<AttError> for crate::Error {
    fn from(e: AttError) -> Self {
        NativeError::GattError(e).into()
    }
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Self {
        Error {
            kind,
            source: None,
            message: String::new(),
        }
    }
}

impl From<NativeError> for Error {
    fn from(err: NativeError) -> Self {
        use BluetoothStatusCode::*;
        let kind = match &err {
            NativeError::GattError(att_error) => ErrorKind::Protocol(*att_error),
            NativeError::BluetoothStatusCode(code) => match code {
                NotAllowed => ErrorKind::NotAuthorized,
                NotEnabled => ErrorKind::AdapterUnavailable,
                NotBonded => ErrorKind::NotAuthorized,
                GattWriteNotAllowed => ErrorKind::NotAuthorized,
                GattWriteBusy => ErrorKind::NotReady,
                MissingBluetoothConnectPermission => ErrorKind::NotAuthorized,
                ProfileServiceNotBound => ErrorKind::Other,
                Unknown => ErrorKind::Other,
                FeatureNotSupported => ErrorKind::NotSupported,
                UnknownError(_) => ErrorKind::Other,
            },
            NativeError::JavaError(_) | NativeError::JavaCallReturnedFalse => ErrorKind::Internal,
        };
        let msg = err.to_string();
        Error::new(kind, Some(err), msg)
    }
}

/// See <https://developer.android.com/reference/android/bluetooth/BluetoothStatusCodes>.
#[derive(Clone, Debug)]
pub enum BluetoothStatusCode {
    NotAllowed,
    NotEnabled,
    NotBonded,
    GattWriteNotAllowed,
    GattWriteBusy,
    MissingBluetoothConnectPermission,
    ProfileServiceNotBound,
    Unknown,
    FeatureNotSupported,
    UnknownError(NonZeroI32),
}

impl std::fmt::Display for BluetoothStatusCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let err_str = match self {
            Self::NotAllowed =>
                "Error code indicating that the API call was initiated by neither the system nor the active user.",
            Self::NotEnabled =>
                "Error code indicating that Bluetooth is not enabled.",
            Self::NotBonded =>
                "Error code indicating that the Bluetooth Device specified is not bonded.",
            Self::GattWriteNotAllowed =>
                "A GATT writeCharacteristic request is not permitted on the remote device.",
            Self::GattWriteBusy =>
                "A GATT writeCharacteristic request is not permitted on the remote device.",
            Self::MissingBluetoothConnectPermission =>
                "Error code indicating that the caller does not have the Manifest.permission.BLUETOOTH_CONNECT permission.",
            Self::ProfileServiceNotBound =>
                "Error code indicating that the profile service is not bound.",
            Self::Unknown =>
                "Indicates that an unknown error has occurred.",
            Self::FeatureNotSupported =>
                "Indicates that the feature is not supported.",
            Self::UnknownError(code) => {
                return f.write_str(&format!("Unknown Error with code {code}"));
            }
        };
        f.write_str(err_str)
    }
}

impl From<NonZeroI32> for BluetoothStatusCode {
    fn from(code: NonZeroI32) -> Self {
        let raw_code = code.get();
        use BluetoothStatusCodes as Codes;
        match raw_code {
            Codes::ERROR_BLUETOOTH_NOT_ALLOWED => Self::NotAllowed,
            Codes::ERROR_BLUETOOTH_NOT_ENABLED => Self::NotEnabled,
            Codes::ERROR_DEVICE_NOT_BONDED => Self::NotBonded,
            Codes::ERROR_GATT_WRITE_NOT_ALLOWED => Self::GattWriteNotAllowed,
            Codes::ERROR_GATT_WRITE_REQUEST_BUSY => Self::GattWriteBusy,
            Codes::ERROR_MISSING_BLUETOOTH_CONNECT_PERMISSION => {
                Self::MissingBluetoothConnectPermission
            }
            Codes::ERROR_PROFILE_SERVICE_NOT_BOUND => Self::ProfileServiceNotBound,
            Codes::ERROR_UNKNOWN => Self::Unknown,
            Codes::FEATURE_NOT_SUPPORTED => Self::FeatureNotSupported,
            _ => Self::UnknownError(code),
        }
    }
}

// NOTE: Code below is migrated from <https://docs.rs/bluest/0.6.9/src/bluest/error.rs.html>.

/// The error type for Bluetooth operations.
#[derive(Clone, Debug)]
pub struct Error {
    kind: ErrorKind,
    source: Option<NativeError>,
    message: String,
}

impl Error {
    pub(crate) fn new<S: ToString>(
        kind: ErrorKind,
        source: Option<NativeError>,
        message: S,
    ) -> Self {
        Error {
            kind,
            source,
            message: message.to_string(),
        }
    }

    /// Returns the corresponding [`ErrorKind`] for this error.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns the message for this error.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns a cloned source error if exists.
    pub fn source_cloned(&self) -> Option<Box<dyn std::error::Error + Send + Sync + 'static>> {
        self.source.clone().map(|e| Box::new(e) as _)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.message.is_empty(), &self.source) {
            (true, None) => write!(f, "{}", &self.kind),
            (false, None) => write!(f, "{}: {}", &self.kind, &self.message),
            (_, Some(err)) => write!(f, "{}", err), // MODIFIED
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|x| {
            let x: &(dyn std::error::Error + 'static) = x;
            x
        })
    }
}

/// A list of general categories of Bluetooth error.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ErrorKind {
    /// the Bluetooth adapter is not available
    AdapterUnavailable,
    /// the Bluetooth adapter is already scanning
    AlreadyScanning,
    /// connection failed
    ConnectionFailed,
    /// the Bluetooth device isn't connected
    NotConnected,
    /// the Bluetooth operation is unsupported
    NotSupported,
    /// permission denied
    NotAuthorized,
    /// not ready
    NotReady,
    /// not found
    NotFound,
    /// invalid paramter
    InvalidParameter,
    /// timed out
    Timeout,
    /// protocol error: {0}
    Protocol(AttError),
    /// an internal error has occured
    Internal,
    /// the service changed and is no longer valid
    ServiceChanged,
    /// error
    Other,
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorKind::AdapterUnavailable => f.write_str("the Bluetooth adapter is not available"),
            ErrorKind::AlreadyScanning => f.write_str("the Bluetooth adapter is already scanning"),
            ErrorKind::ConnectionFailed => f.write_str("connection failed"),
            ErrorKind::NotConnected => f.write_str("the Bluetooth device isn't connected"),
            ErrorKind::NotSupported => f.write_str("the Bluetooth operation is unsupported"),
            ErrorKind::NotAuthorized => f.write_str("permission denied"),
            ErrorKind::NotReady => f.write_str("not ready"),
            ErrorKind::NotFound => f.write_str("not found"),
            ErrorKind::InvalidParameter => f.write_str("invalid paramter"),
            ErrorKind::Timeout => f.write_str("timed out"),
            ErrorKind::Protocol(err) => write!(f, "protocol error: {err}"),
            ErrorKind::Internal => f.write_str("an internal error has occured"),
            ErrorKind::ServiceChanged => f.write_str("the service changed and is no longer valid"),
            ErrorKind::Other => f.write_str("error"),
        }
    }
}

/// Bluetooth Attribute Protocol error. See the Bluetooth Core Specification, Vol 3, Part F, §3.4.1.1
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttError(u8);

impl AttError {
    /// The operation completed successfully.
    pub const SUCCESS: AttError = AttError(0x00);
    /// The attribute handle given was not valid on this server.
    pub const INVALID_HANDLE: AttError = AttError(0x01);
    /// The attribute cannot be read.
    pub const READ_NOT_PERMITTED: AttError = AttError(0x02);
    /// The attribute cannot be written.
    pub const WRITE_NOT_PERMITTED: AttError = AttError(0x03);
    /// The attribute PDU was invalid.
    pub const INVALID_PDU: AttError = AttError(0x04);
    /// The attribute requires authentication before it can be read or written.
    pub const INSUFFICIENT_AUTHENTICATION: AttError = AttError(0x05);
    /// Attribute server does not support the request received from the client.
    pub const REQUEST_NOT_SUPPORTED: AttError = AttError(0x06);
    /// Offset specified was past the end of the attribute.
    pub const INVALID_OFFSET: AttError = AttError(0x07);
    /// The attribute requires authorization before it can be read or written.
    pub const INSUFFICIENT_AUTHORIZATION: AttError = AttError(0x08);
    /// Too many prepare writes have been queued.
    pub const PREPARE_QUEUE_FULL: AttError = AttError(0x09);
    /// No attribute found within the given attribute handle range.
    pub const ATTRIBUTE_NOT_FOUND: AttError = AttError(0x0a);
    /// The attribute cannot be read or written using the Read Blob Request.
    pub const ATTRIBUTE_NOT_LONG: AttError = AttError(0x0b);
    /// The Encryption Key Size used for encrypting this link is insufficient.
    pub const INSUFFICIENT_ENCRYPTION_KEY_SIZE: AttError = AttError(0x0c);
    /// The attribute value length is invalid for the operation.
    pub const INVALID_ATTRIBUTE_VALUE_LENGTH: AttError = AttError(0x0d);
    /// The attribute request that was requested has encountered an error that was unlikely, and therefore could not be completed as requested.
    pub const UNLIKELY_ERROR: AttError = AttError(0x0e);
    /// The attribute requires encryption before it can be read or written.
    pub const INSUFFICIENT_ENCRYPTION: AttError = AttError(0x0f);
    /// The attribute type is not a supported grouping attribute as defined by a higher layer specification.
    pub const UNSUPPORTED_GROUP_TYPE: AttError = AttError(0x10);
    /// Insufficient Resources to complete the request.
    pub const INSUFFICIENT_RESOURCES: AttError = AttError(0x11);
    /// The server requests the client to rediscover the database.
    pub const DATABASE_OUT_OF_SYNC: AttError = AttError(0x12);
    /// The attribute parameter value was not allowed.
    pub const VALUE_NOT_ALLOWED: AttError = AttError(0x13);
    /// Write Request Rejected
    pub const WRITE_REQUEST_REJECTED: AttError = AttError(0xfc);
    /// Client Characteristic Configuration Descriptor Improperly Configured
    pub const CCCD_IMPROPERLY_CONFIGURED: AttError = AttError(0xfd);
    /// Procedure Already in Progress
    pub const PROCEDURE_ALREADY_IN_PROGRESS: AttError = AttError(0xfe);
    /// Out of Range
    pub const OUT_OF_RANGE: AttError = AttError(0xff);

    /// Converts a [`u8`] value to an [`AttError`].
    pub const fn from_u8(val: u8) -> Self {
        AttError(val)
    }

    /// Converts an [`AttError`] to a [`u8`] value.
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Checks if the error code is in the application error range.
    pub fn is_application(&self) -> bool {
        (0x80..0xa0).contains(&self.0)
    }

    /// Checks if the error code is in the common profile and service range.
    pub fn is_common_profile_or_service(&self) -> bool {
        self.0 >= 0xe0
    }
}

impl std::fmt::Display for AttError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            AttError::SUCCESS => f.write_str("The operation completed successfully."),
            AttError::INVALID_HANDLE => f.write_str("The attribute handle given was not valid on this server."),
            AttError::READ_NOT_PERMITTED => f.write_str("The attribute cannot be read."),
            AttError::WRITE_NOT_PERMITTED => f.write_str("The attribute cannot be written."),
            AttError::INVALID_PDU => f.write_str("The attribute PDU was invalid."),
            AttError::INSUFFICIENT_AUTHENTICATION => f.write_str("The attribute requires authentication before it can be read or written."),
            AttError::REQUEST_NOT_SUPPORTED => f.write_str("Attribute server does not support the request received from the client."),
            AttError::INVALID_OFFSET => f.write_str("Offset specified was past the end of the attribute."),
            AttError::INSUFFICIENT_AUTHORIZATION => f.write_str("The attribute requires authorization before it can be read or written."),
            AttError::PREPARE_QUEUE_FULL => f.write_str("Too many prepare writes have been queued."),
            AttError::ATTRIBUTE_NOT_FOUND => f.write_str("No attribute found within the given attribute handle range."),
            AttError::ATTRIBUTE_NOT_LONG => f.write_str("The attribute cannot be read or written using the Read Blob Request."),
            AttError::INSUFFICIENT_ENCRYPTION_KEY_SIZE => f.write_str("The Encryption Key Size used for encrypting this link is insufficient."),
            AttError::INVALID_ATTRIBUTE_VALUE_LENGTH => f.write_str("The attribute value length is invalid for the operation."),
            AttError::UNLIKELY_ERROR => f.write_str("The attribute request that was requested has encountered an error that was unlikely, and therefore could not be completed as requested."),
            AttError::INSUFFICIENT_ENCRYPTION => f.write_str("The attribute requires encryption before it can be read or written."),
            AttError::UNSUPPORTED_GROUP_TYPE => f.write_str("The attribute type is not a supported grouping attribute as defined by a higher layer specification."),
            AttError::INSUFFICIENT_RESOURCES => f.write_str("Insufficient Resources to complete the request."),
            AttError::DATABASE_OUT_OF_SYNC => f.write_str("The server requests the client to rediscover the database."),
            AttError::VALUE_NOT_ALLOWED => f.write_str("The attribute parameter value was not allowed."),
            AttError::WRITE_REQUEST_REJECTED => f.write_str("Write Request Rejected"),
            AttError::CCCD_IMPROPERLY_CONFIGURED => f.write_str("Client Characteristic Configuration Descriptor Improperly Configured"),
            AttError::PROCEDURE_ALREADY_IN_PROGRESS => f.write_str("Procedure Already in Progress"),
            AttError::OUT_OF_RANGE => f.write_str("Out of Range"),
            _ => f.write_str(&format!("Unknown error 0x{:02x}", self.0)),
        }
    }
}

impl From<u8> for AttError {
    fn from(number: u8) -> Self {
        AttError(number)
    }
}

impl From<AttError> for u8 {
    fn from(val: AttError) -> Self {
        val.0
    }
}
