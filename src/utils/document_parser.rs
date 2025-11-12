use std::borrow::Cow;
use std::io::{Cursor, Read};

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use calamine::{Data, Reader, Xlsx};
use quick_xml::events::Event;
use quick_xml::reader::Reader as XmlReader;
use tracing::info;
use zip::read::ZipArchive;

/// 支持的文档类型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DocumentType {
    Pdf,
    Docx,
    Txt,
    Md,
    Xlsx,
}

impl DocumentType {
    /// 从文件名推断文档类型
    pub fn from_filename(filename: &str) -> Option<Self> {
        let lower = filename.to_lowercase();
        if lower.ends_with(".pdf") {
            Some(Self::Pdf)
        } else if lower.ends_with(".docx") {
            Some(Self::Docx)
        } else if lower.ends_with(".txt") || lower.ends_with(".json") || lower.ends_with(".csv") {
            Some(Self::Txt)
        } else if lower.ends_with(".md") {
            Some(Self::Md)
        } else if lower.ends_with(".xlsx") {
            Some(Self::Xlsx)
        } else {
            None
        }
    }

    /// 获取文档类型的描述
    pub fn description(&self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Docx => "Word (DOCX)",
            Self::Txt => "Text (TXT)",
            Self::Md => "Markdown",
            Self::Xlsx => "Excel (XLSX)",
        }
    }
}

/// 解析文档内容
pub struct DocumentParser;

impl DocumentParser {
    /// 解析文档字节流，返回纯文本内容
    pub async fn parse(filename: &str, data: Bytes) -> Result<String> {
        let doc_type = DocumentType::from_filename(filename)
            .ok_or_else(|| anyhow!("Unsupported file type: {}", filename))?;

        info!("Parsing {} as {}", filename, doc_type.description());

        match doc_type {
            DocumentType::Pdf => Self::parse_pdf(&data),
            DocumentType::Docx => Self::parse_docx_md(&data),
            DocumentType::Txt | DocumentType::Md => Self::parse_text(&data),
            DocumentType::Xlsx => Self::parse_xlsx(&data),
        }
    }

    /// 解析 DOCX 文件（支持表格识别）
    fn parse_docx_md(data: &[u8]) -> Result<String> {
        let cursor = Cursor::new(data);
        let mut archive = ZipArchive::new(cursor).context("无法打开docx文件")?;

        // 读取 document.xml
        let mut document_xml = archive
            .by_name("word/document.xml")
            .context("找不到document.xml")?;

        let mut xml_content = Vec::new();
        document_xml
            .read_to_end(&mut xml_content)
            .context("读取document.xml失败")?;

        // 解析 XML
        Self::parse_docx_xml(&xml_content)
    }

    /// 解析 DOCX XML 内容
    fn parse_docx_xml(xml: &[u8]) -> Result<String> {
        let mut reader = XmlReader::from_reader(Cursor::new(xml));
        reader.config_mut().trim_text(true);

        // 预分配容量以减少重新分配
        let mut result = Vec::with_capacity(128);
        let mut current_paragraph = String::with_capacity(512);
        let mut current_table: Vec<Vec<String>> = Vec::with_capacity(32);
        let mut current_row: Vec<String> = Vec::with_capacity(16);
        let mut current_cell = String::with_capacity(128);
        let mut in_table = false;
        let mut in_row = false;
        let mut in_cell = false;
        let mut in_field = false; // 是否在域代码中
        let mut buf = Vec::with_capacity(256);

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let name = e.local_name();
                    let name_bytes = name.as_ref();

                    // 检测域代码开始
                    if name_bytes.ends_with(b"fldChar") {
                        // 检查是否是域开始
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref().ends_with(b"fldCharType") {
                                let value = attr.value;
                                if value.as_ref() == b"begin" {
                                    in_field = true;
                                } else if value.as_ref() == b"end" {
                                    in_field = false;
                                }
                            }
                        }
                    } else if name_bytes.ends_with(b"instrText")
                        || name_bytes.ends_with(b"fldSimple")
                    {
                        // 域指令文本或简单域，标记为在域中
                        in_field = true;
                    } else if name_bytes.ends_with(b"tbl") {
                        // 表格开始
                        in_table = true;
                        current_table.clear();
                    } else if name_bytes.ends_with(b"tr") {
                        // 行开始
                        in_row = true;
                        current_row.clear();
                    } else if name_bytes.ends_with(b"tc") {
                        // 单元格开始
                        in_cell = true;
                        current_cell.clear();
                    } else if name_bytes.ends_with(b"p") && !in_table {
                        // 段落开始（非表格内）
                        current_paragraph.clear();
                    }
                }
                Ok(Event::End(ref e)) => {
                    let name = e.local_name();
                    let name_bytes = name.as_ref();

                    // 域指令文本结束
                    if name_bytes.ends_with(b"instrText") || name_bytes.ends_with(b"fldSimple") {
                        in_field = false;
                    } else if name_bytes.ends_with(b"tbl") {
                        // 表格结束，转换为 Markdown
                        if !current_table.is_empty() {
                            let markdown_table = Self::table_to_markdown(&current_table);
                            result.push(markdown_table);
                            result.push(String::new()); // 空行
                        }
                        in_table = false;
                    } else if name_bytes.ends_with(b"tr") {
                        // 行结束
                        if in_row && !current_row.is_empty() {
                            current_table.push(current_row.clone());
                        }
                        in_row = false;
                    } else if name_bytes.ends_with(b"tc") {
                        // 单元格结束
                        if in_cell {
                            current_row.push(current_cell.trim().to_string());
                        }
                        in_cell = false;
                    } else if name_bytes.ends_with(b"p") && !in_table {
                        // 段落结束（非表格内）
                        if !current_paragraph.is_empty() {
                            let trimmed = current_paragraph.trim();
                            if !trimmed.is_empty() {
                                result.push(trimmed.to_string());
                            }
                            current_paragraph.clear();
                        }
                    }
                }
                Ok(Event::Text(e)) => {
                    // 跳过域代码中的文本
                    if in_field {
                        continue;
                    }

                    // 使用 Cow 避免不必要的分配
                    let text = e.decode().unwrap_or_default();

                    if in_cell {
                        current_cell.push_str(&text);
                    } else if !in_table {
                        current_paragraph.push_str(&text);
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(anyhow!("XML解析错误: {:?}", e)),
                _ => {}
            }
            buf.clear();
        }

        Ok(result.join("\n"))
    }

    /// 将表格数据转换为 Markdown 表格
    fn table_to_markdown(table: &[Vec<String>]) -> String {
        if table.is_empty() {
            return String::new();
        }

        // 获取最大列数
        let max_cols = table.iter().map(|row| row.len()).max().unwrap_or(0);

        if max_cols == 0 {
            return String::new();
        }

        // 预估容量：每行约 150 字符
        let estimated_capacity = table.len() * 150 * max_cols;
        let mut result = String::with_capacity(estimated_capacity);

        // 生成表格行
        for (idx, row) in table.iter().enumerate() {
            result.push_str("| ");

            // 输出单元格
            for col_idx in 0..max_cols {
                if col_idx > 0 {
                    result.push_str(" | ");
                }

                if let Some(cell) = row.get(col_idx) {
                    // 转义 Markdown 特殊字符 - 使用 Cow 避免不必要的分配
                    let escaped = if cell.contains('|') || cell.contains('\n') {
                        Cow::Owned(cell.replace('|', "\\|").replace('\n', " "))
                    } else {
                        Cow::Borrowed(cell.as_str())
                    };
                    result.push_str(&escaped);
                }
            }

            result.push_str(" |\n");

            // 第一行后添加分隔线
            if idx == 0 {
                result.push_str("| ");
                for col_idx in 0..max_cols {
                    if col_idx > 0 {
                        result.push_str(" | ");
                    }
                    result.push_str("---");
                }
                result.push_str(" |\n");
            }
        }

        result
    }

    /// 解析 XLSX 文件，输出为 Markdown 格式
    fn parse_xlsx(data: &[u8]) -> Result<String> {
        // 使用 Cursor 将字节数组包装成可读流
        let cursor = Cursor::new(data);

        // 打开 xlsx 工作簿
        let mut workbook: Xlsx<_> =
            Xlsx::new(cursor).map_err(|e| anyhow!("Failed to parse XLSX: {:?}", e))?;

        // 预分配容量
        let mut all_text = String::with_capacity(4096);

        // 获取所有工作表名称
        let sheet_names = workbook.sheet_names().to_vec();

        // 遍历每个工作表
        for sheet_name in sheet_names {
            // 读取工作表范围数据
            if let Ok(range) = workbook.worksheet_range(&sheet_name) {
                // 添加工作表标题（Markdown H2）
                all_text.push_str("\n## ");
                all_text.push_str(&sheet_name);
                all_text.push_str("\n\n");

                let rows: Vec<Vec<String>> = range
                    .rows()
                    .map(|row| row.iter().map(Self::cell_to_string).collect())
                    .collect();

                if rows.is_empty() {
                    continue;
                }

                // 获取最大列数
                let max_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);

                if max_cols == 0 {
                    continue;
                }

                // 生成 Markdown 表格 - 直接写入 String
                for (idx, row) in rows.iter().enumerate() {
                    all_text.push_str("| ");

                    for col_idx in 0..max_cols {
                        if col_idx > 0 {
                            all_text.push_str(" | ");
                        }

                        if let Some(cell) = row.get(col_idx) {
                            all_text.push_str(cell);
                        }
                    }

                    all_text.push_str(" |\n");

                    // 第一行后添加分隔线
                    if idx == 0 {
                        all_text.push_str("| ");
                        for col_idx in 0..max_cols {
                            if col_idx > 0 {
                                all_text.push_str(" | ");
                            }
                            all_text.push_str("---");
                        }
                        all_text.push_str(" |\n");
                    }
                }

                // 工作表之间添加空行
                all_text.push('\n');
            }
        }

        let result = all_text;

        if result.trim().is_empty() {
            Err(anyhow!("XLSX 文件为空或无法提取文本"))
        } else {
            Ok(result)
        }
    }

    /// 格式化文本为 Markdown 格式
    fn format_text_to_markdown(text: &str) -> String {
        // 预分配容量
        let mut result = String::with_capacity(text.len() + text.len() / 10);
        let mut prev_line_empty = false;
        let mut is_first_line = true;

        for line in text.lines() {
            let trimmed = line.trim();

            // 跳过多余的空行
            if trimmed.is_empty() {
                if !prev_line_empty {
                    if !is_first_line {
                        result.push('\n');
                    }
                    prev_line_empty = true;
                }
                continue;
            }

            // 检测可能的标题（短行且不以标点结尾）
            let is_potential_title = trimmed.len() < 60
                && !trimmed.ends_with('。')
                && !trimmed.ends_with('.')
                && !trimmed.ends_with(',')
                && !trimmed.ends_with('，')
                && !trimmed.ends_with('：')
                && !trimmed.ends_with(':')
                && !trimmed.starts_with('-')
                && !trimmed.starts_with('*')
                && !trimmed.starts_with('#')
                && prev_line_empty;

            if !is_first_line {
                result.push('\n');
            }

            if is_potential_title {
                result.push_str("## ");
            }

            result.push_str(trimmed);

            prev_line_empty = false;
            is_first_line = false;
        }

        result
    }

    /// 将单元格转换为字符串
    fn cell_to_string(cell: &Data) -> String {
        match cell {
            Data::Empty => String::new(),
            Data::String(s) => s.replace('|', "\\|"), // 转义 Markdown 表格分隔符
            Data::Float(f) => {
                // 格式化浮点数，避免过长的小数
                if f.fract() == 0.0 {
                    format!("{:.0}", f)
                } else {
                    format!("{:.2}", f)
                }
            }
            Data::Int(i) => i.to_string(),
            Data::Bool(b) => if *b { "✓" } else { "✗" }.to_string(),
            Data::Error(e) => format!("ERROR: {:?}", e),
            Data::DateTime(dt) => format!("{:.0}", dt),
            Data::DateTimeIso(dt) => dt.to_string(),
            Data::DurationIso(d) => d.to_string(),
        }
    }

    /// 解析 PDF 文件，输出为 Markdown 格式
    fn parse_pdf(data: &[u8]) -> Result<String> {
        // 使用 pdf-extract 解析 PDF
        match pdf_extract::extract_text_from_mem(data) {
            Ok(text) => {
                if text.trim().is_empty() {
                    Err(anyhow!("PDF 文件为空或无法提取文本"))
                } else {
                    // 格式化为 Markdown
                    let formatted = Self::format_text_to_markdown(&text);
                    Ok(formatted)
                }
            }
            Err(e) => Err(anyhow!("Failed to parse PDF: {}", e)),
        }
    }

    /// 解析纯文本文件（支持多种编码：UTF-8, GBK等）
    fn parse_text(data: &[u8]) -> Result<String> {
        // 尝试检测和解码文本
        let text = Self::decode_text(data)?;

        // 格式化为 Markdown
        let formatted = Self::format_text_to_markdown(&text);
        Ok(formatted)
    }

    /// 智能检测和解码文本（支持UTF-8, GBK等编码）
    fn decode_text(data: &[u8]) -> Result<String> {
        // 1. 首先尝试UTF-8
        if let Ok(text) = std::str::from_utf8(data) {
            info!("文本编码: UTF-8");
            return Ok(text.to_string());
        }

        // 2. 使用编码检测器自动检测编码
        let mut detector = chardetng::EncodingDetector::new();
        detector.feed(data, true);
        let encoding = detector.guess(None, true);

        info!("检测到的编码: {}", encoding.name());

        // 3. 尝试使用检测到的编码解码
        let (decoded, encoding_used, had_errors) = encoding.decode(data);

        if had_errors {
            return Err(anyhow!(
                "无法解码文本文件，尝试的编码: {}",
                encoding_used.name()
            ));
        }

        Ok(decoded.into_owned())
    }

    /// 获取支持的文件扩展名列表
    pub fn supported_extensions() -> Vec<&'static str> {
        vec![".pdf", ".docx", ".xlsx", ".txt", ".md", "json", "csv"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_type_detection() {
        assert_eq!(
            DocumentType::from_filename("test.pdf"),
            Some(DocumentType::Pdf)
        );
        assert_eq!(
            DocumentType::from_filename("test.PDF"),
            Some(DocumentType::Pdf)
        );
        assert_eq!(
            DocumentType::from_filename("test.docx"),
            Some(DocumentType::Docx)
        );
        assert_eq!(
            DocumentType::from_filename("test.txt"),
            Some(DocumentType::Txt)
        );
        assert_eq!(
            DocumentType::from_filename("test.md"),
            Some(DocumentType::Md)
        );
        assert_eq!(DocumentType::from_filename("test.unknown"), None);
    }

    #[tokio::test]
    async fn test_parse_docx() {
        let content = std::fs::read("/Users/anger/Downloads/file/硬件需求--MySQL版.docx").unwrap();
        let result = DocumentParser::parse("test.docx", Bytes::from(content))
            .await
            .unwrap();
        println!("{}", result);
        assert!(!result.is_empty());
    }

    #[test]
    fn test_decode_utf8_text() {
        let utf8_text = "你好，世界！Hello World!";
        let utf8_bytes = utf8_text.as_bytes();

        let result = DocumentParser::decode_text(utf8_bytes).unwrap();
        assert_eq!(result, utf8_text);
    }

    #[test]
    fn test_decode_gbk_text() {
        use encoding_rs::GBK;

        // 创建GBK编码的测试数据
        let original_text = "你好，这是GBK编码的中文文本！";
        let (gbk_bytes, _, _) = GBK.encode(original_text);

        // 测试解码
        let result = DocumentParser::decode_text(&gbk_bytes).unwrap();
        assert_eq!(result, original_text);
    }
}
