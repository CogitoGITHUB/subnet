use crate::query_web::css;
use scraper::{Html, Selector as ScraperSelector, element_ref::ElementRef};
use std::collections::HashMap;

pub type Headers = HashMap<String, usize>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebTable {
    headers: Headers,
    pub data: Vec<Vec<String>>,
}

impl WebTable {
    /// Finds the first table in `html`.
    pub fn find_first(html: &str) -> Option<WebTable> {
        let html = Html::parse_fragment(html);
        html.select(&css("table", false)).next().map(WebTable::new)
    }

    pub fn find_all_tables(html: &str) -> Option<Vec<WebTable>> {
        let html = Html::parse_fragment(html);
        let iter: Vec<WebTable> = html
            .select(&css("table", false))
            .map(WebTable::new)
            .collect();
        if iter.is_empty() {
            return None;
        }
        Some(iter)
    }

    /// Finds the table in `html` with an id of `id`.
    pub fn find_by_id(html: &str, id: &str) -> Option<WebTable> {
        let html = Html::parse_fragment(html);
        let selector = format!("table#{id}");
        ScraperSelector::parse(&selector)
            .ok()
            .as_ref()
            .map(|s| html.select(s))
            .and_then(|mut s| s.next())
            .map(WebTable::new)
    }

    /// Finds the table in `html` whose first row contains all of the headers
    /// specified in `headers`. The order does not matter.
    ///
    /// If `headers` is empty, this is the same as
    /// [`find_first`](#method.find_first).
    pub fn find_by_headers<T>(
        html: &str,
        headers: &[T],
        inspect_mode: bool,
    ) -> Option<Vec<WebTable>>
    where
        T: AsRef<str>,
    {
        if headers.is_empty() {
            return WebTable::find_all_tables(html);
        }

        let sel_table = css("table", false);
        let sel_tr = css("tr", false);
        let sel_th = css("th", false);

        let html = Html::parse_fragment(html);
        let mut tables = html
            .select(&sel_table)
            .filter(|table| {
                table.select(&sel_tr).next().is_some_and(|tr| {
                    let cells = select_cells(tr, &sel_th, true);
                    if inspect_mode {
                        eprintln!("Potential HTML Headers = {:?}\n", cells);
                    }
                    headers.iter().all(|h| contains_str(&cells, h.as_ref()))
                })
            })
            .peekable();
        tables.peek()?;
        Some(tables.map(WebTable::new).collect())
    }

    /// Returns the headers of the table.
    ///
    /// This will be empty if the table had no `<th>` tags in its first row. See
    /// [`Headers`](type.Headers.html) for more.
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    /// Returns an iterator over the [`Row`](struct.Row.html)s of the table.
    ///
    /// Only `<td>` cells are considered when generating rows. If the first row
    /// of the table is a header row, meaning it contains at least one `<th>`
    /// cell, the iterator will start on the second row. Use
    /// [`headers`](#method.headers) to access the header row in that case.
    pub fn iter(&self) -> Iter<'_> {
        Iter {
            headers: &self.headers,
            iter: self.data.iter(),
        }
    }

    pub fn empty() -> WebTable {
        WebTable {
            headers: HashMap::new(),
            data: vec![vec!["".to_string()]],
        }
    }

    // fn new(element: ElementRef) -> Table {
    //     let sel_tr = css("tr", false);
    //     let sel_th = css("th", false);
    //     let sel_td = css("td", false);

    //     let mut headers = HashMap::new();
    //     let mut rows = element.select(&sel_tr).peekable();
    //     if let Some(tr) = rows.peek() {
    //         for (i, th) in tr.select(&sel_th).enumerate() {
    //             headers.insert(cell_content(th), i);
    //         }
    //     }
    //     if !headers.is_empty() {
    //         rows.next();
    //     }
    //     let data = rows.map(|tr| select_cells(tr, &sel_td, true)).collect();
    //     Table { headers, data }
    // }

    fn new(element: ElementRef) -> WebTable {
        let sel_tr = css("tr", false);
        let sel_th = css("th", false);
        let sel_td = css("td", false);

        let mut headers = HashMap::new();
        let mut rows = element.select(&sel_tr).peekable();
        if let Some(tr) = rows.clone().peek() {
            for (i, th) in tr.select(&sel_th).enumerate() {
                headers.insert(cell_content(th), i);
            }
        }
        if !headers.is_empty() {
            rows.next();
        }

        if headers.is_empty() {
            // try looking for data as headers i.e. they're row headers not column headers
            for (i, d) in rows
                .clone()
                .map(|tr| select_cells(tr, &sel_th, true))
                .enumerate()
            {
                headers.insert(d.join(", "), i);
            }
            // check if headers are there but empty
            let mut empty_headers = true;
            for (h, _i) in headers.clone() {
                if !h.is_empty() {
                    empty_headers = false;
                    break;
                }
            }
            if empty_headers {
                headers = HashMap::new();
            }
            let data = rows.map(|tr| select_cells(tr, &sel_td, true)).collect();
            WebTable { headers, data }
        } else {
            let data = rows.map(|tr| select_cells(tr, &sel_td, true)).collect();
            WebTable { headers, data }
        }
    }
}

impl<'a> IntoIterator for &'a WebTable {
    type Item = Row<'a>;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over the rows in a [`Table`](struct.Table.html).
pub struct Iter<'a> {
    headers: &'a Headers,
    iter: std::slice::Iter<'a, Vec<String>>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = Row<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let headers = self.headers;
        self.iter.next().map(|cells| Row { headers, cells })
    }
}

/// A row in a [`Table`](struct.Table.html).
///
/// A row consists of a number of data cells stored as strings. If the row
/// contains the same number of cells as the table's header row, its cells can
/// be safely accessed by header names using [`get`](#method.get). Otherwise,
/// the data should be accessed via [`as_slice`](#method.as_slice) or by
/// iterating over the row.
///
/// This struct can be thought of as a lightweight reference into a table. As
/// such, it implements the `Copy` trait.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Row<'a> {
    headers: &'a Headers,
    cells: &'a [String],
}

impl<'a> Row<'a> {
    /// Returns the number of cells in the row.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Returns `true` if the row contains no cells.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Returns the cell underneath `header`.
    ///
    /// Returns `None` if there is no such header, or if there is no cell at
    /// that position in the row.
    pub fn get(&self, header: &str) -> Option<&'a str> {
        // eprintln!(
        //     "header={}, headers={:?}, cells={:?}",
        //     &header, &self.headers, &self.cells
        // );
        self.headers.get(header).and_then(|&i| {
            // eprintln!("i={}", i);
            self.cells.get(i).map(String::as_str)
        })
    }

    pub fn get_header_at(&self, index: usize) -> Option<&'a str> {
        let mut a_match = "";
        for (key, val) in self.headers {
            if *val == index {
                a_match = key;
                break;
            }
        }
        if a_match.is_empty() {
            None
        } else {
            Some(a_match)
        }
    }

    /// Returns a slice containing all the cells.
    pub fn as_slice(&self) -> &'a [String] {
        self.cells
    }

    /// Returns an iterator over the cells of the row.
    pub fn iter(&self) -> std::slice::Iter<'_, String> {
        self.cells.iter()
    }
}

impl<'a> IntoIterator for Row<'a> {
    type Item = &'a String;
    type IntoIter = std::slice::Iter<'a, String>;

    fn into_iter(self) -> Self::IntoIter {
        self.cells.iter()
    }
}

fn select_cells(
    element: ElementRef,
    selector: &ScraperSelector,
    remove_html_tags: bool,
) -> Vec<String> {
    if remove_html_tags {
        let scraped = element.select(selector).map(cell_content);
        let mut dehtmlized: Vec<String> = Vec::new();
        for item in scraped {
            if item.is_empty() {
                dehtmlized.push(item);
                continue;
            }
            let frag = Html::parse_fragment(&item);
            for node in frag.tree {
                if let scraper::node::Node::Text(text) = node {
                    dehtmlized.push(text.text.to_string());
                }
            }
        }
        dehtmlized
    } else {
        element.select(selector).map(cell_content).collect()
    }
}

fn cell_content(element: ElementRef) -> String {
    // element.inner_html().trim().to_string()
    let mut dehtmlize = String::new();
    let element = element.inner_html().trim().to_string();
    let frag = Html::parse_fragment(&element);
    for node in frag.tree {
        if let scraper::node::Node::Text(text) = node {
            dehtmlize.push_str(&text.text)
        }
    }

    // eprintln!("element={} dehtmlize={}", &element, &dehtmlize);

    if dehtmlize.is_empty() {
        dehtmlize = element;
    }

    dehtmlize
}

fn contains_str(slice: &[String], item: &str) -> bool {
    // slice.iter().any(|s| s == item)

    let mut dehtmlized = String::new();
    let frag = Html::parse_fragment(item);
    for node in frag.tree {
        if let scraper::node::Node::Text(text) = node {
            dehtmlized.push_str(&text.text);
        }
    }

    if dehtmlized.is_empty() {
        dehtmlized = item.to_string();
    }

    slice.iter().any(|s| {
        // eprintln!(
        //     "\ns={} item={} contains={}\n",
        //     &s,
        //     &dehtmlized,
        //     &dehtmlized.contains(s)
        // );
        // s.starts_with(item)
        dehtmlized.contains(s)
    })
}

/*
    #[test]
    fn test_wikipedia_swapped_rows_columns() {
        // empty columns
        let cols = console_protocol::value::Value {
            value: console_protocol::UntaggedValue::Primitive(console_protocol::Primitive::String(
                "".to_string(),
            )),
            tag: shell_source::Tag::unknown(),
        };

        // this table is taken straight from wikipedia with no changes
        let table = retrieve_tables(HTML_TABLE_WIKIPEDIA_COLUMNS_AS_ROWS, &cols, true);

        let expected = vec![UntaggedValue::row(indexmap! {
                 "Stable release".to_string() => UntaggedValue::string("\n          2103 (16.0.13901.20400) / April\u{a0}13, 2021; 4 months ago\u{a0}(2021-04-13)[1]\n        ").into(),
                 "Developer(s)".to_string() => UntaggedValue::string("Microsoft").into(),
                 "Operating system".to_string() => UntaggedValue::string("Microsoft Windows").into(),
                 "Type".to_string() => UntaggedValue::string("Spreadsheet").into(),
                 "License".to_string() => UntaggedValue::string("Trialware[2]").into(),
                 "".to_string() => UntaggedValue::string("").into(),
                 "Website".to_string() => UntaggedValue::string("products.office.com/en-us/excel").into(),
                 "Initial release".to_string() => UntaggedValue::string("1987; 34\u{a0}years ago\u{a0}(1987)").into(),
             }).into()];

        assert_eq!(table, expected);
    }

    #[test]
    fn test_wikipedia_table_with_column_headers() {
        let cols = UntaggedValue::table(&[
            UntaggedValue::string("Format".to_string()).into(),
            UntaggedValue::string("Extension".to_string()).into(),
            UntaggedValue::string("Description".to_string()).into(),
        ])
        .into();

        // this table is taken straight from wikipedia with no changes
        let table = retrieve_tables(HTML_TABLE_WIKIPEDIA_WITH_COLUMN_NAMES, &cols, true);
        let expected = vec![
                 UntaggedValue::row(indexmap! {
                     "Format".to_string() => UntaggedValue::string("Excel Workbook").into(),
                     "Extension".to_string() => UntaggedValue::string(".xlsx").into(),
                     "Description".to_string() => UntaggedValue::string("The default Excel 2007 and later workbook format. In reality, a Zip compressed archive with a directory structure of XML text documents. Functions as the primary

    //replacement for the former binary .xls format, although it does not support Excel macros for security reasons. Saving as .xlsx offers file size reduction over .xls[38]").into(),
                 }).into(),
                 UntaggedValue::row(indexmap! {
                     "Format".to_string() => UntaggedValue::string("Excel Macro-enabled Workbook").into(),
                     "Extension".to_string() => UntaggedValue::string(".xlsm").into(),
                     "Description".to_string() => UntaggedValue::string("As Excel Workbook, but with macro support.").into(),
                 }).into(),
                 UntaggedValue::row(indexmap! {
                     "Format".to_string() => UntaggedValue::string("Excel Binary Workbook").into(),
                     "Extension".to_string() => UntaggedValue::string(".xlsb").into(),
                     "Description".to_string() => UntaggedValue::string("As Excel Macro-enabled Workbook, but storing information in binary form rather than XML documents for opening and saving documents more quickly and efficiently. Intended especially for very large documents with tens of thousands of rows, and/or several hundreds of columns. This format is very useful for shrinking large Excel files as is often the case when doing data analysis.").into(),
                 }).into(),
                 UntaggedValue::row(indexmap! {
                     "Format".to_string() => UntaggedValue::string("Excel Macro-enabled Template").into(),
                     "Extension".to_string() => UntaggedValue::string(".xltm").into(),
                     "Description".to_string() => UntaggedValue::string("A template document that forms a basis for actual workbooks, with macro support. The replacement for the old .xlt format.").into(),
                 }).into(),
                 UntaggedValue::row(indexmap! {
                     "Format".to_string() => UntaggedValue::string("Excel Add-in").into(),
                     "Extension".to_string() => UntaggedValue::string(".xlam").into(),
                     "Description".to_string() => UntaggedValue::string("Excel add-in to add extra functionality and tools. Inherent macro support because of the file purpose.").into(),
                 }).into(),
             ];

        assert_eq!(table, expected);
    }
*/
