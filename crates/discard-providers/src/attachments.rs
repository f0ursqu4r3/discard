use discard_core::{attachments::Attachment, provider::ProviderError};
use serde_json::{Value, json};

pub(crate) fn codex_input(text: &str, images: &[Attachment]) -> Result<Vec<Value>, ProviderError> {
    let mut input = vec![json!({"type": "text", "text": text})];
    for image in images {
        input.push(json!({"type": "text", "text": format!("Attached image: {} (ID {})", image.name, image.id)}));
        input.push(json!({"type": "image", "url": image.data_url().map_err(ProviderError::Io)?}));
    }
    Ok(input)
}
pub(crate) fn claude_content(
    text: &str,
    images: &[Attachment],
) -> Result<Vec<Value>, ProviderError> {
    let mut content = Vec::new();
    for image in images {
        let url = image.data_url().map_err(ProviderError::Io)?;
        let (_, data) = url
            .split_once(',')
            .ok_or_else(|| ProviderError::Protocol("Invalid image data.".into()))?;
        content.push(json!({"type": "text", "text": format!("Attached image: {} (ID {})", image.name, image.id)}));
        content.push(json!({"type": "image", "source": {"type": "base64", "media_type": image.media_type, "data": data}}));
    }
    content.push(json!({"type": "text", "text": text}));
    Ok(content)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_image_inputs_include_content_and_identity_for_both_providers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image");
        std::fs::write(&path, b"image bytes").unwrap();
        let attachment = Attachment {
            id: "image-id".into(),
            name: "diagram.png".into(),
            media_type: "image/png".into(),
            size: 11,
            path,
            text: None,
        };
        let codex = codex_input("Review", &[attachment.clone()]).unwrap();
        assert_eq!(codex[0]["text"], "Review");
        assert!(codex[1]["text"].as_str().unwrap().contains("diagram.png"));
        assert_eq!(codex[2]["type"], "image");
        assert_eq!(codex[2]["url"], "data:image/png;base64,aW1hZ2UgYnl0ZXM=");
        let claude = claude_content("Review", &[attachment]).unwrap();
        assert_eq!(claude[1]["source"]["media_type"], "image/png");
        assert_eq!(claude[1]["source"]["data"], "aW1hZ2UgYnl0ZXM=");
        assert_eq!(claude[2]["text"], "Review");
    }
}
