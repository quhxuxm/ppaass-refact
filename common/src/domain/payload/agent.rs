use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::error::CommonError;

use super::common::NetAddress;

/// The agent message body type
#[derive(Debug)]
pub enum AgentMessagePayloadType {
    /// The tcp connect message
    TcpConnect,
    /// The tcp connection close message
    TcpConnectionClose,
    /// The tcp data message
    TcpData,
    /// The udp associate message
    UdpAssociate,
    /// The udp data message
    UdpData,
}

/// Convert a agent message payload type to u8
impl From<AgentMessagePayloadType> for u8 {
    fn from(value: AgentMessagePayloadType) -> Self {
        match value {
            AgentMessagePayloadType::TcpConnect => 10,
            AgentMessagePayloadType::TcpData => 11,
            AgentMessagePayloadType::TcpConnectionClose => 12,
            AgentMessagePayloadType::UdpAssociate => 20,
            AgentMessagePayloadType::UdpData => 21,
        }
    }
}

/// Convert a u8 to agent message payload type.
impl TryFrom<u8> for AgentMessagePayloadType {
    type Error = CommonError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            10 => Ok(AgentMessagePayloadType::TcpConnect),
            11 => Ok(AgentMessagePayloadType::TcpData),
            12 => Ok(AgentMessagePayloadType::TcpConnectionClose),
            20 => Ok(AgentMessagePayloadType::UdpAssociate),
            21 => Ok(AgentMessagePayloadType::UdpData),
            _ => Err(CommonError::UnknownPayloadType(value)),
        }
    }
}

/// The agent message payload
#[derive(Debug)]
pub struct PpaassAgentMessagePayload {
    /// The source address
    source_address: NetAddress,
    /// The target address
    target_address: NetAddress,
    /// The payload type
    payload_type: AgentMessagePayloadType,
    /// The data
    data: Bytes,
}

#[derive(Debug)]
pub struct PpaassAgentMessagePayloadSplitResult {
    /// The source address
    pub source_address: PpaassAddress,
    /// The target address
    pub target_address: PpaassAddress,
    /// The payload type
    pub payload_type: AgentMessagePayloadType,
    /// The data
    pub data: Bytes,
}

impl PpaassAgentMessagePayload {
    pub fn new(
        source_address: PpaassAddress,
        target_address: PpaassAddress,
        payload_type: AgentMessagePayloadType,
        data: Bytes,
    ) -> Self {
        PpaassAgentMessagePayload {
            source_address,
            target_address,
            payload_type,
            data,
        }
    }

    pub fn split(self) -> PpaassAgentMessagePayloadSplitResult {
        PpaassAgentMessagePayloadSplitResult {
            source_address: self.source_address,
            target_address: self.target_address,
            payload_type: self.payload_type,
            data: self.data,
        }
    }
}

impl From<PpaassAgentMessagePayload> for Bytes {
    fn from(value: PpaassAgentMessagePayload) -> Self {
        let mut result = BytesMut::new();
        result.put_u8(value.payload_type.into());
        let source_address: Vec<u8> = value.source_address.into();
        let source_address_length = source_address.len();
        result.put_u64(source_address_length as u64);
        result.put_slice(source_address.as_slice());
        let target_address: Vec<u8> = value.target_address.into();
        let target_address_length = target_address.len();
        result.put_u64(target_address_length as u64);
        result.put_slice(target_address.as_slice());
        result.put_u64(value.data.len() as u64);
        result.put(value.data);
        result.into()
    }
}

impl TryFrom<Bytes> for PpaassAgentMessagePayload {
    type Error = PpaassCommonError;

    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let mut bytes = value;
        let payload_type: AgentMessagePayloadType = bytes.get_u8().try_into()?;
        let source_address_length = bytes.get_u64() as usize;
        let source_address_bytes = bytes.copy_to_bytes(source_address_length);
        let source_address = source_address_bytes.to_vec().try_into()?;
        let target_address_length = bytes.get_u64() as usize;
        let target_address_bytes = bytes.copy_to_bytes(target_address_length);
        let target_address = target_address_bytes.to_vec().try_into()?;
        let data_length = bytes.get_u64() as usize;
        let data = bytes.copy_to_bytes(data_length);
        Ok(Self {
            payload_type,
            source_address,
            target_address,
            data,
        })
    }
}
