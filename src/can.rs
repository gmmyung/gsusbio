//! Validated, transport-independent CAN frames.

use arrayvec::ArrayVec;

#[cfg(all(feature = "can-fd", any(feature = "tokio", feature = "blocking", test)))]
use crate::constants::GS_CAN_FLAG_ESI;
#[cfg(any(feature = "tokio", feature = "blocking", test))]
use crate::constants::{CAN_EFF_FLAG, CAN_ERR_FLAG, CAN_RTR_FLAG, GS_CAN_FLAG_BRS, GS_CAN_FLAG_FD};
use crate::constants::{CAN_EFF_MASK, CAN_SFF_MASK};
#[cfg(any(feature = "tokio", feature = "blocking", test))]
use crate::frame::GsUsbFrame;
use crate::socket_error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CanId {
    Standard(u16),
    Extended(u32),
}

impl CanId {
    pub fn standard(id: u16) -> Result<Self> {
        if u32::from(id) > CAN_SFF_MASK {
            return Err(Error::InvalidFrame("standard CAN ID exceeds 11 bits"));
        }
        Ok(Self::Standard(id))
    }

    pub fn extended(id: u32) -> Result<Self> {
        if id > CAN_EFF_MASK {
            return Err(Error::InvalidFrame("extended CAN ID exceeds 29 bits"));
        }
        Ok(Self::Extended(id))
    }

    pub fn value(self) -> u32 {
        match self {
            Self::Standard(id) => u32::from(id),
            Self::Extended(id) => id,
        }
    }

    pub fn is_extended(self) -> bool {
        matches!(self, Self::Extended(_))
    }

    fn valid(self) -> bool {
        match self {
            Self::Standard(id) => u32::from(id) <= CAN_SFF_MASK,
            Self::Extended(id) => id <= CAN_EFF_MASK,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanFrame {
    ClassicData(ClassicData),
    ClassicRemote(ClassicRemote),
    #[cfg(feature = "can-fd")]
    Fd(FdFrame),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicData {
    id: CanId,
    data: ArrayVec<u8, 8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicRemote {
    id: CanId,
    dlc: u8,
}

#[cfg(feature = "can-fd")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FdFrame {
    id: CanId,
    data: ArrayVec<u8, 64>,
    brs: bool,
    esi: Option<bool>,
}

impl CanFrame {
    pub fn classic(id: CanId, data: &[u8]) -> Result<Self> {
        if !id.valid() || data.len() > 8 {
            return Err(Error::InvalidFrame(
                "invalid classic CAN ID or payload length",
            ));
        }
        Ok(Self::ClassicData(ClassicData {
            id,
            data: ArrayVec::try_from(data)
                .map_err(|_| Error::InvalidFrame("payload exceeds 8 bytes"))?,
        }))
    }

    pub fn remote(id: CanId, dlc: u8) -> Result<Self> {
        if !id.valid() || dlc > 8 {
            return Err(Error::InvalidFrame("invalid remote CAN ID or DLC"));
        }
        Ok(Self::ClassicRemote(ClassicRemote { id, dlc }))
    }

    #[cfg(feature = "can-fd")]
    pub fn fd(id: CanId, data: &[u8], brs: bool) -> Result<Self> {
        if !id.valid() || fd_len_to_dlc(data.len()).is_none() {
            return Err(Error::InvalidFrame("invalid CAN FD ID or payload length"));
        }
        Ok(Self::Fd(FdFrame {
            id,
            data: ArrayVec::try_from(data)
                .map_err(|_| Error::InvalidFrame("payload exceeds 64 bytes"))?,
            brs,
            esi: None,
        }))
    }

    pub fn id(&self) -> CanId {
        match self {
            Self::ClassicData(v) => v.id,
            Self::ClassicRemote(v) => v.id,
            #[cfg(feature = "can-fd")]
            Self::Fd(v) => v.id,
        }
    }

    pub fn data(&self) -> &[u8] {
        match self {
            Self::ClassicData(v) => &v.data,
            Self::ClassicRemote(_) => &[],
            #[cfg(feature = "can-fd")]
            Self::Fd(v) => &v.data,
        }
    }

    pub fn dlc(&self) -> u8 {
        match self {
            Self::ClassicData(v) => v.data.len() as u8,
            Self::ClassicRemote(v) => v.dlc,
            #[cfg(feature = "can-fd")]
            Self::Fd(v) => fd_len_to_dlc(v.data.len()).expect("validated CAN FD length"),
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Self::ClassicRemote(_))
    }

    pub fn is_fd(&self) -> bool {
        #[cfg(feature = "can-fd")]
        {
            matches!(self, Self::Fd(_))
        }
        #[cfg(not(feature = "can-fd"))]
        {
            false
        }
    }

    pub fn brs(&self) -> bool {
        #[cfg(feature = "can-fd")]
        {
            matches!(self, Self::Fd(v) if v.brs)
        }
        #[cfg(not(feature = "can-fd"))]
        {
            false
        }
    }

    pub fn esi(&self) -> Option<bool> {
        #[cfg(feature = "can-fd")]
        {
            match self {
                Self::Fd(v) => v.esi,
                _ => None,
            }
        }
        #[cfg(not(feature = "can-fd"))]
        {
            None
        }
    }

    #[cfg(any(feature = "tokio", feature = "blocking", test))]
    pub(crate) fn to_raw(&self, channel: u8, echo_id: u32) -> Result<GsUsbFrame> {
        if !self.id().valid() {
            return Err(Error::InvalidFrame("CAN ID is out of range"));
        }
        let mut raw = GsUsbFrame::new();
        raw.echo_id = echo_id;
        raw.channel = channel;
        raw.can_id = self.id().value();
        if self.id().is_extended() {
            raw.can_id |= CAN_EFF_FLAG;
        }
        raw.can_dlc = self.dlc();
        raw.data[..self.data().len()].copy_from_slice(self.data());
        if self.is_remote() {
            raw.can_id |= CAN_RTR_FLAG;
        }
        if self.is_fd() {
            raw.flags |= GS_CAN_FLAG_FD;
        }
        if self.brs() {
            raw.flags |= GS_CAN_FLAG_BRS;
        }
        if self.esi() == Some(true) {
            return Err(Error::InvalidFrame("transmitting ESI is unsupported"));
        }
        Ok(raw)
    }

    #[cfg(any(feature = "tokio", feature = "blocking", test))]
    pub(crate) fn from_raw(raw: &GsUsbFrame) -> Result<Self> {
        if raw.can_id & CAN_ERR_FLAG != 0 {
            return Err(Error::Protocol("CAN error frame is not a data frame"));
        }
        if raw.can_id & CAN_EFF_FLAG == 0 && raw.can_id & CAN_EFF_MASK > CAN_SFF_MASK {
            return Err(Error::Protocol("standard CAN ID exceeds 11 bits"));
        }
        if raw.flags & GS_CAN_FLAG_FD == 0 && raw.can_dlc > 8 {
            return Err(Error::Protocol("classic CAN DLC exceeds 8"));
        }
        if raw.flags & GS_CAN_FLAG_FD != 0 && raw.can_id & CAN_RTR_FLAG != 0 {
            return Err(Error::Protocol("CAN FD cannot be a remote frame"));
        }
        let id = if raw.can_id & CAN_EFF_FLAG != 0 {
            CanId::extended(raw.can_id & CAN_EFF_MASK)?
        } else {
            CanId::standard((raw.can_id & CAN_SFF_MASK) as u16)?
        };
        if raw.flags & GS_CAN_FLAG_FD != 0 {
            #[cfg(feature = "can-fd")]
            {
                let mut frame = Self::fd(id, raw.data(), raw.flags & GS_CAN_FLAG_BRS != 0)?;
                if let Self::Fd(v) = &mut frame {
                    v.esi = Some(raw.flags & GS_CAN_FLAG_ESI != 0);
                }
                return Ok(frame);
            }
            #[cfg(not(feature = "can-fd"))]
            return Err(Error::UnsupportedFeature(
                "CAN FD is disabled at compile time",
            ));
        }
        if raw.can_id & CAN_RTR_FLAG != 0 {
            Self::remote(id, raw.can_dlc)
        } else {
            Self::classic(id, raw.data())
        }
    }
}

#[cfg(feature = "can-fd")]
fn fd_len_to_dlc(len: usize) -> Option<u8> {
    const LENGTHS: [usize; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64];
    LENGTHS.iter().position(|&n| n == len).map(|n| n as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_ids_and_frame_lengths() {
        assert!(CanId::standard(0x800).is_err());
        assert!(CanId::extended(0x2000_0000).is_err());
        assert!(CanFrame::classic(CanId::standard(0x123).unwrap(), &[0; 9]).is_err());
        assert!(CanFrame::remote(CanId::standard(0x123).unwrap(), 9).is_err());
        #[cfg(feature = "can-fd")]
        assert!(CanFrame::fd(CanId::standard(0x123).unwrap(), &[0; 11], true).is_err());
    }

    #[test]
    fn preserves_extended_id_and_remote_dlc() {
        let frame = CanFrame::remote(CanId::extended(0x123).unwrap(), 8).unwrap();
        let raw = frame.to_raw(2, 7).unwrap();
        assert_eq!(raw.channel, 2);
        assert_eq!(raw.echo_id, 7);
        assert_eq!(CanFrame::from_raw(&raw).unwrap(), frame);
    }

    #[cfg(feature = "can-fd")]
    #[test]
    fn round_trips_fd_lengths() {
        for length in [0, 1, 8, 12, 16, 20, 24, 32, 48, 64] {
            let frame =
                CanFrame::fd(CanId::extended(0x123).unwrap(), &vec![3; length], true).unwrap();
            let raw = frame.to_raw(0, 1).unwrap();
            let decoded = CanFrame::from_raw(&raw).unwrap();
            assert_eq!(decoded.data(), frame.data());
            assert_eq!(decoded.dlc(), frame.dlc());
        }
    }
}
