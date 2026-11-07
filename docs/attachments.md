# Message attachments

Use **Attach files**, drag files anywhere onto the active conversation's main panel, or paste clipboard images into the message field.
The full main panel highlights the file drop target; previews appear in the composer. Drop individual files; folders are not supported.
Normal text paste stays unchanged. All three methods use the same limits and compact preview cards above the message field.
Image cards show thumbnails. Other cards show a file icon, filename, and size. Select a card to open
an image or text preview. PDF previews show extracted text. Use **Remove** to remove a file from the draft.
Messages can contain text, attachments, or both.

Attachment drafts stay with their channel while the app remains open. A rejected send keeps the draft.
Successful sends retain the attachment cards in message history, including after an app restart.

## Supported files and limits

- PNG, JPEG, GIF, and WebP images.
- UTF-8 text and code files.
- PDFs with selectable text. Locked PDFs and scans without text are rejected. Attach page images for scans.
- Up to eight files per message, five MB per file, and twenty MB total.
- Extracted text must fit within 128 KB per file. The selected model's conversation budget also applies.

## Storage and provider delivery

Discard copies selected files into `$DISCARD_HOME/attachments/<id>/file` and stores metadata beside each
copy. Filenames cannot determine the destination path. Frontend commands use validated opaque IDs.
The original files are not modified. Saved messages contain typed attachment blocks with their metadata
and extracted text. Copies are retained in app storage; removing a card removes it from the draft.

Text and extracted PDF content are sent as reference data. Images use native image inputs for both
Codex and Claude. This does not enable file-reading tools in direct messages. Conversation follow-ups
resend image content when the corresponding attachment remains in unsummarized history. Images from
summarized messages are not resent. Retained image content is limited to twenty MB per request; start a
new conversation or use smaller images if that limit is reached.

The main webview uses HTML file drop events (`dragDropEnabled: false`) so the composer receives browser
file objects. Drop and clipboard imports send bounded base64 file bytes to the native importer; they do
not grant access to client-supplied filesystem paths. The importer validates content and rolls back a
failed batch. Drops outside the main panel do not navigate away from the app.

The preview loads images through a bounded native command as data URLs. No broad filesystem or asset
URL access is granted to the webview. The file picker, image preview, and send operations report errors
without replacing the current draft.
