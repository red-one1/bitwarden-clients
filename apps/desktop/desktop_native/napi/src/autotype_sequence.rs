// Fork: KeePass-style auto-type of the item selected in the desktop app.
#[napi]
pub mod autotype_sequence {
    use autotype::sequence::{Modifiers, SequenceStep};

    /// One resolved step of an auto-type sequence, built by the renderer.
    #[napi(object)]
    pub struct AutotypeStep {
        /// One of `text`, `key`, `char`, `delay` or `keyDelay`.
        pub kind: String,
        /// The text for `text`, or the single character for `char`.
        pub text: Option<String>,
        /// The Windows virtual key code for `key`.
        pub vk: Option<u32>,
        pub shift: Option<bool>,
        pub ctrl: Option<bool>,
        pub alt: Option<bool>,
        pub win: Option<bool>,
        /// Milliseconds for `delay` and `keyDelay`.
        pub ms: Option<u32>,
    }

    impl TryFrom<AutotypeStep> for SequenceStep {
        type Error = napi::Error;

        fn try_from(step: AutotypeStep) -> napi::Result<Self> {
            let modifiers = Modifiers {
                shift: step.shift.unwrap_or(false),
                ctrl: step.ctrl.unwrap_or(false),
                alt: step.alt.unwrap_or(false),
                win: step.win.unwrap_or(false),
            };
            let missing = |field: &str| {
                napi::Error::from_reason(format!(
                    "Auto-type step '{}' is missing '{field}'",
                    step.kind
                ))
            };
            Ok(match step.kind.as_str() {
                "text" => SequenceStep::Text(step.text.clone().ok_or_else(|| missing("text"))?),
                "key" => {
                    let vk = step.vk.ok_or_else(|| missing("vk"))?;
                    let vk = u16::try_from(vk)
                        .map_err(|_| napi::Error::from_reason(format!("Invalid key code {vk}")))?;
                    SequenceStep::Key { vk, modifiers }
                }
                "char" => {
                    let text = step.text.clone().ok_or_else(|| missing("text"))?;
                    let mut chars = text.chars();
                    match (chars.next(), chars.next()) {
                        (Some(ch), None) => SequenceStep::Char { ch, modifiers },
                        _ => return Err(missing("a single character")),
                    }
                }
                "delay" => SequenceStep::Delay(step.ms.ok_or_else(|| missing("ms"))?),
                "keyDelay" => SequenceStep::KeyDelay(step.ms.ok_or_else(|| missing("ms"))?),
                other => {
                    return Err(napi::Error::from_reason(format!(
                        "Unknown auto-type step '{other}'"
                    )))
                }
            })
        }
    }

    /// Switches to the window below Bitwarden (like KeePass) and types the steps into it.
    #[napi]
    pub async fn perform(steps: Vec<AutotypeStep>) -> napi::Result<()> {
        let steps = steps
            .into_iter()
            .map(SequenceStep::try_from)
            .collect::<napi::Result<Vec<_>>>()?;

        tokio::task::spawn_blocking(move || {
            autotype::sequence::activate_previous_window()?;
            autotype::sequence::type_sequence(&steps)
        })
        .await
        .map_err(|e| napi::Error::from_reason(e.to_string()))??;

        Ok(())
    }
}
