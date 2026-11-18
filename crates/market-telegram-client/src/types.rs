use url::Url;
#[derive(Debug, Clone, Copy)]
pub struct IncomingMessage<'a> {
    pub private: bool,
    pub outgoing: bool,
    pub user_id: Option<u64>,
    pub message_id: i32,
    pub text: Option<&'a str>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuCommand {
    pub name: &'static str,
    pub description: &'static str,
}
#[derive(Debug, Clone)]
pub struct LaunchButton {
    pub text: String,
    pub url: Url,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub text: String,
}
impl Reply {
    pub fn bounded(mut self) -> Self {
        if self.text.trim().is_empty() {
            self.text = "This command produced no result.".into();
        }
        if self.text.encode_utf16().count() > 4096 {
            let mut units = 0;
            self.text = self
                .text
                .chars()
                .take_while(|ch| {
                    units += ch.len_utf16();
                    units <= 4095
                })
                .collect();
            self.text.push('…');
        }
        self
    }
}
