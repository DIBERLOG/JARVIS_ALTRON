use serde::Deserialize;
use std::io::{Cursor, Write};

#[derive(Deserialize)]
pub struct ExportRun { text:String, #[serde(default)] bold:bool, #[serde(default)] italic:bool }
#[derive(Deserialize)]
pub struct ExportBlock {
    kind:String,
    #[serde(default)] level:u8,
    #[serde(default)] runs:Vec<ExportRun>,
    #[serde(default)] rows:Vec<Vec<Vec<ExportRun>>>,
}
fn xml(text:&str)->String {
    text.replace('&',"&amp;").replace('<',"&lt;").replace('>',"&gt;").replace('"',"&quot;").replace('\'',"&apos;")
}
fn paragraph(runs:&[ExportRun],level:u8)->String {
    let properties=if level>0 {format!("<w:keepNext/><w:outlineLvl w:val=\"{}\"/>",level-1)} else {String::new()};
    let mut result=format!("<w:p><w:pPr>{properties}<w:spacing w:after=\"120\"/></w:pPr>");
    for run in runs {
        result.push_str(&format!("<w:r><w:rPr>{}{}<w:color w:val=\"000000\"/><w:sz w:val=\"{}\"/></w:rPr>",if run.bold||level>0 {"<w:b/>"} else {""},if run.italic {"<w:i/>"} else {""},if level>0 {28} else {22}));
        for (i,line) in run.text.split('\n').enumerate() {
            if i>0 {result.push_str("<w:br/>");}
            result.push_str(&format!("<w:t xml:space=\"preserve\">{}</w:t>",xml(line)));
        }
        result.push_str("</w:r>");
    }
    result.push_str("</w:p>");result
}
fn build_docx(blocks:Vec<ExportBlock>)->Result<Vec<u8>,String> {
    if blocks.is_empty()||blocks.len()>10000 {return Err("Документ пустой или содержит слишком много блоков.".into());}
    let mut size=0usize;
    for block in &blocks {
        if !matches!(block.kind.as_str(),"paragraph"|"table") || block.level>3 || block.rows.len()>2000 || block.rows.iter().any(|row|row.is_empty()||row.len()>30) {return Err("Некорректная структура документа или таблица шире 30 столбцов.".into());}
        for run in block.runs.iter().chain(block.rows.iter().flatten().flatten()) {
            size=size.saturating_add(run.text.len());
            if run.text.chars().any(|c|c<' ' && !matches!(c,'\n'|'\r'|'\t')) {return Err("Текст содержит недопустимые символы.".into());}
        }
    }
    if size>1_000_000 {return Err("Документ превышает 1 МБ текста. Сократите результат.".into());}
    let mut body=String::new();
    for block in blocks {
        if block.kind=="paragraph" {body.push_str(&paragraph(&block.runs,block.level));continue;}
        let columns=block.rows.first().map_or(0,Vec::len);
        if columns==0||block.rows.iter().any(|row|row.len()!=columns) {return Err("Таблица содержит строки с разным числом столбцов.".into());}
        let width=9360/columns;
        body.push_str("<w:tbl><w:tblPr><w:tblW w:w=\"9360\" w:type=\"dxa\"/><w:tblBorders>");
        for edge in ["top","left","bottom","right","insideH","insideV"] {body.push_str(&format!("<w:{edge} w:val=\"single\" w:sz=\"4\" w:color=\"A4B4BB\"/>"));}
        body.push_str("</w:tblBorders><w:tblCellMar><w:top w:w=\"100\" w:type=\"dxa\"/><w:left w:w=\"100\" w:type=\"dxa\"/><w:bottom w:w=\"100\" w:type=\"dxa\"/><w:right w:w=\"100\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr><w:tblGrid>");
        for _ in 0..columns {body.push_str(&format!("<w:gridCol w:w=\"{width}\"/>"));}
        body.push_str("</w:tblGrid>");
        for (index,row) in block.rows.into_iter().enumerate() {
            body.push_str(if index==0 {"<w:tr><w:trPr><w:tblHeader/></w:trPr>"} else {"<w:tr>"});
            for mut cell in row {
                if index==0 {for run in &mut cell {run.bold=true;}}
                body.push_str(&format!("<w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"dxa\"/>{}</w:tcPr>{}</w:tc>",if index==0 {"<w:shd w:fill=\"E8EFF2\"/>"} else {""},paragraph(&cell,0)));
            }
            body.push_str("</w:tr>");
        }
        body.push_str("</w:tbl><w:p/>");
    }
    let document=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}<w:sectPr><w:pgSz w:w=\"12240\" w:h=\"15840\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\"/></w:sectPr></w:body></w:document>");
    let mut archive=zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options=zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let parts=[("[Content_Types].xml",r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#),
        ("_rels/.rels",r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#),
        ("word/document.xml",document.as_str())];
    for (name,content) in parts {archive.start_file(name,options).map_err(|_|"Не удалось создать Word-файл.")?;archive.write_all(content.as_bytes()).map_err(|_|"Не удалось записать Word-файл.")?;}
    Ok(archive.finish().map_err(|_|"Не удалось завершить Word-файл.")?.into_inner())
}
#[tauri::command]
pub fn agent_export_docx(blocks:Vec<ExportBlock>)->Result<Vec<u8>,String> {build_docx(blocks)}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn agent_export_word_has_native_table_and_escaped_text() {
        use std::io::Read;
        let blocks=serde_json::from_value(serde_json::json!([
            {"kind":"paragraph","level":1,"runs":[{"text":"План задач"}]},
            {"kind":"table","rows":[[[{"text":"Задача"}],[{"text":"Исполнитель"}]],[[{"text":"<Проверить> & договор"}],[{"text":"Иван"}]]]}
        ])).unwrap();
        let bytes=build_docx(blocks).unwrap();
        let mut archive=zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let mut content=String::new();archive.by_name("word/document.xml").unwrap().read_to_string(&mut content).unwrap();
        assert!(content.contains("<w:tbl>"));assert!(content.contains("<w:tblHeader/>"));assert!(content.contains("&lt;Проверить&gt; &amp; договор"));
        assert!(build_docx(vec![]).is_err());
        if let Ok(path)=std::env::var("JARVIS_EXPORT_QA_DOCX") {std::fs::write(path,bytes).unwrap();}
    }
}
