use dhcproto::{Decodable, Decoder, Encodable, Encoder, v6::Message};

pub struct DhcpV6Codec {}

impl tokio_util::codec::Decoder for DhcpV6Codec {
    type Item = dhcproto::v6::Message;

    type Error = dhcproto::v6::DecodeError;

    fn decode(
        &mut self,
        src: &mut tokio_util::bytes::BytesMut,
    ) -> Result<Option<Self::Item>, Self::Error> {
        let mut dec = Decoder::new(dbg!(&src));
        match Message::decode(&mut dec) {
            Ok(e) => {
                *src = dec.buffer().into();
                Ok(Some(e))
            }
            Err(dhcproto::v6::DecodeError::NotEnoughBytes) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl tokio_util::codec::Encoder<dhcproto::v6::Message> for DhcpV6Codec {
    type Error = dhcproto::v6::EncodeError;

    fn encode(
        &mut self,
        item: dhcproto::v6::Message,
        dst: &mut tokio_util::bytes::BytesMut,
    ) -> Result<(), Self::Error> {
        let mut buffer = Vec::new();
        let mut encoder = Encoder::new(&mut buffer);
        item.encode(&mut encoder).unwrap();
        dst.extend_from_slice(&buffer);
        Ok(())
    }
}
