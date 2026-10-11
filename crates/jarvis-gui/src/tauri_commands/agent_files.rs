use quick_xml::{events::Event, Reader};
use serde::Serialize;
use std::io::{Cursor, Read};

const MAX_FILE_BYTES: usize = 5 * 1024 * 1024;
const MAX_XML_BYTES: u64 = 4 * 1024 * 1024;
const MAX_TEXT_CHARS: usize = 8000;

#[derive(Serialize)]
pub struct AgentAttachment {
    name: String,
    text: String,
    truncated: bool,
    warning: String,
}

fn text_file(data: &[u8]) -> Result<String, String> {
    let text = if data.starts_with(&[0xff, 0xfe]) || data.starts_with(&[0xfe, 0xff]) {
        if data.len() % 2 != 0 { return Err("Повреждённый текст UTF-16.".into()); }
        let little = data[0] == 0xff;
        let words: Vec<u16> = data[2..].chunks_exact(2).map(|p| if little {u16::from_le_bytes([p[0],p[1]])} else {u16::from_be_bytes([p[0],p[1]])}).collect();
        String::from_utf16(&words).map_err(|_| "Не удалось прочитать текст UTF-16.")?
    } else {
        std::str::from_utf8(data).map_err(|_| "Сохраните текстовый файл в UTF-8 или UTF-16 с BOM и прикрепите снова.")?.trim_start_matches('\u{feff}').to_owned()
    };
    if text.chars().any(|c| c.is_control() && !matches!(c, '\n'|'\r'|'\t')) {
        return Err("Файл содержит двоичные данные, а не обычный текст.".into());
    }
    Ok(text)
}

fn docx_text(data: &[u8]) -> Result<String, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(data)).map_err(|_| "Не удалось прочитать DOCX. Проверьте, что файл не повреждён и не защищён паролем.")?;
    let entry = archive.by_name("word/document.xml").map_err(|_| "В файле нет текста документа Word. Используйте DOCX, а не DOC или DOCM.")?;
    if entry.size() > MAX_XML_BYTES { return Err("Текст внутри DOCX слишком большой. Прикрепите небольшой фрагмент документа.".into()); }
    let mut xml = String::new();
    entry.take(MAX_XML_BYTES + 1).read_to_string(&mut xml).map_err(|_| "Не удалось извлечь текст DOCX.")?;
    if xml.len() as u64 > MAX_XML_BYTES || xml.contains("<!DOCTYPE") { return Err("Неподдерживаемое содержимое DOCX.".into()); }
    let mut reader = Reader::from_str(&xml);
    let mut text = String::new();
    loop {
        match reader.read_event().map_err(|_| "Повреждённая структура DOCX.")? {
            Event::Start(e) if e.local_name().as_ref() == b"t" => {
                let raw = reader.read_text(e.name()).map_err(|_| "Не удалось прочитать текст DOCX.")?;
                text.push_str(&quick_xml::escape::unescape(&raw).map_err(|_| "Некорректный текст DOCX.")?);
            }
            Event::Empty(e) if matches!(e.local_name().as_ref(), b"tab") => text.push('\t'),
            Event::Empty(e) if matches!(e.local_name().as_ref(), b"br"|b"cr") => text.push('\n'),
            Event::End(e) if e.local_name().as_ref() == b"p" => text.push('\n'),
            Event::End(e) if e.local_name().as_ref() == b"tc" => text.push('\t'),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(text)
}

fn extract_attachment(name: String, data: Vec<u8>) -> Result<AgentAttachment, String> {
    if name.is_empty() || name.len() > 500 || name.contains(['/', '\\']) || data.is_empty() || data.len() > MAX_FILE_BYTES {
        return Err("Выберите непустой документ размером до 5 МБ.".into());
    }
    let extension = name.rsplit('.').next().unwrap_or("").to_lowercase();
    let (text, warning) = match extension.as_str() {
        "docx" => (docx_text(&data)?, "Извлечён основной текст и текст таблиц. Изображения, колонтитулы, примечания и оформление не передаются.".into()),
        "txt"|"md"|"csv"|"json"|"log" => (text_file(&data)?, String::new()),
        _ => return Err("Поддерживаются DOCX, TXT, MD, CSV, JSON и LOG. PDF, изображения, DOC и XLSX пока преобразуйте в текст или DOCX.".into()),
    };
    let text = text.trim();
    if text.is_empty() { return Err("В документе не найден текст. Изображения и сканы пока не распознаются.".into()); }
    let truncated = text.chars().count() > MAX_TEXT_CHARS;
    Ok(AgentAttachment { name, text:text.chars().take(MAX_TEXT_CHARS).collect(), truncated, warning })
}

#[tauri::command]
pub async fn agent_read_attachment(name: String, data: Vec<u8>) -> Result<AgentAttachment, String> {
    // Reads supplied bytes only: no paths, file execution, macros or external document links.
    tauri::async_runtime::spawn_blocking(move || extract_attachment(name, data)).await
        .map_err(|_| "Не удалось прочитать документ.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn agent_attachments_extract_validate_and_report_truncation() {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        archive.start_file("word/document.xml", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
        archive.write_all(br#"<w:document xmlns:w="word"><w:p><w:r><w:t>Hello &amp; world</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Price 10</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:document>"#).unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        let result = extract_attachment("test.docx".into(), bytes).unwrap();
        assert!(result.text.contains("Hello & world\n"));
        assert!(result.text.contains("Price 10"));
        assert!(!result.warning.is_empty());
        assert!(extract_attachment("test.pdf".into(), b"%PDF".to_vec()).is_err());
        assert!(extract_attachment("test.docx".into(), b"broken".to_vec()).is_err());
        assert!(extract_attachment("test.txt".into(), vec![0,1]).is_err());
        assert!(extract_attachment("test.txt".into(), vec![b'a'; MAX_FILE_BYTES+1]).is_err());
        let long = extract_attachment("long.txt".into(), "Я".repeat(9000).into_bytes()).unwrap();
        assert!(long.truncated); assert_eq!(long.text.chars().count(),8000);
        assert_eq!(text_file(&[0xff,0xfe,0x1f,0x04,0x40,0x04]).unwrap(),"Пр");
    }
}
