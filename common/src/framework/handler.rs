use async_trait::async_trait;

use crate::framework::domain::Channel;
use crate::Message;

#[async_trait]
pub trait ChannelHandler<InputMessage, OutputMessage> {
    async fn on_accept(mut channel: Channel);

    async fn on_read(mut channel: Channel, input: InputMessage);

    async fn on_write(mut channel: Channel, output: OutputMessage);

    async fn on_close(mut channel: Channel);
}
