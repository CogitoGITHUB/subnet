use crate::Query;
use console_plugin::{EngineInterface, EvaluatedCall, SimplePluginCommand};
use console_protocol::{
    Category, Example, LabeledError, Record, Signature, Span, Spanned, SyntaxShape, Type, Value,
    record,
};
use sxd_document::parser;
use sxd_xpath::{Context, Factory};

pub struct QueryXml;

impl SimplePluginCommand for QueryXml {
    type Plugin = Query;

    fn name(&self) -> &str {
        "query xml"
    }

    fn description(&self) -> &str {
        "Execute XPath 1.0 query on XML input"
    }

    fn extra_description(&self) -> &str {
        r#"Scalar results (Number, String, Boolean) are returned as nu scalars.
Output of the nodeset results depends on the flags used:
    - No flags: returns a table with `string_value` column.
    - You have to specify `--output-string-value` to include `string_value` in the output when using any other `--output-*` flags.
    - `--output-type` includes `type` column with node type.
    - `--output-names` includes `local_name`, `prefixed_name`, and `namespace` columns.
        "#
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .required("query", SyntaxShape::String, "Xpath query.")
            .named(
                "namespaces",
                SyntaxShape::record(),
                "Map of prefixes to namespace URIs.",
                Some('n'),
            )
            .switch(
                "output-string-value",
                "Include `string_value` in the nodeset output. On by default.",
                None,
            )
            .switch(
                "output-type",
                "Include `type` in the nodeset output. Off by default.",
                None,
            )
            .switch(
                "output-names",
                "Include `local_name`, `prefixed_name`, and `namespace` in the nodeset output. Off by default.",
                None,
            )
            .input_output_types(vec![
                (Type::String, Type::Any),
            ])
            .category(Category::Filters)
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            // full output
            Example {
                description: "Query namespaces on the root element of an SVG file",
                example: r#"http get --raw https://www.w3.org/TR/SVG/images/conform/smiley.svg
    | query xml '/svg:svg/namespace::*' --output-string-value --output-names --output-type --namespaces {svg: "http://www.w3.org/2000/svg"}"#,
                result: None,
            },
            // scalar output
            Example {
                description: "Query the language of Nushell blog (`xml:` prefix is always available)",
                example: r#"http get --raw https://www.nushell.sh/atom.xml
    | query xml 'string(/*/@xml:lang)'"#,
                result: None,
            },
            // query attributes
            Example {
                description: "Query all XLink targets in SVG document",
                example: r#"http get --raw https://www.w3.org/TR/SVG/images/conform/smiley.svg
    | query xml '//*/@xlink:href' --namespaces {xlink: "http://www.w3.org/1999/xlink"}"#,
                result: None,
            },
            // default output
            Example {
                description: "Get recent Nushell news",
                example: r#"http get --raw https://www.nushell.sh/atom.xml
    | query xml '//atom:entry/atom:title|//atom:entry/atom:link/@href' --namespaces {atom: "http://www.w3.org/2005/Atom"}
    | window 2 --stride 2
    | each { {title: $in.0.string_value, link: $in.1.string_value} }"#,
                result: None,
            },
        ]
    }

    fn run(
        &self,
        _plugin: &Query,
        _engine: &EngineInterface,
        call: &EvaluatedCall,
        input: &Value,
    ) -> Result<Value, LabeledError> {
        let query: Option<Spanned<String>> = call.opt(0)?;
        let namespaces: Option<Record> = call.get_flag::<Record>("namespaces")?;

        execute_xpath_query(call, input, query, namespaces)
    }
}

pub fn execute_xpath_query(
    call: &EvaluatedCall,
    input: &Value,
    query: Option<Spanned<String>>,
    namespaces: Option<Record>,
) -> Result<Value, LabeledError> {
    let (query_string, span) = match &query {
        Some(v) => (&v.item, v.span),
        None => {
            return Err(
                LabeledError::new("problem with input data").with_label("query missing", call.head)
            );
        }
    };

    let node_output_options = NodeOutputOptions::from_call(call);

    let xpath = build_xpath(query_string, span)?;
    let input_string = input.coerce_str()?;
    let package = parser::parse(&input_string);

    if let Err(err) = package {
        return Err(
            LabeledError::new("Invalid XML document").with_label(err.to_string(), input.span())
        );
    }

    let package = package.expect("invalid xml document");

    let document = package.as_document();
    let mut context = Context::new();

    let mut namespaces = namespaces.unwrap_or_default();

    if namespaces.get("xml").is_none() {
        // XML namespace is always present, so we add it explicitly
        // it's used in attributes like `xml:lang`, `xml:base`, etc.
        namespaces.insert(
            "xml",
            Value::string("http://www.w3.org/XML/1998/namespace", call.head),
        );
    }

    // NB: `xmlns:whatever=` or `xmlns=` may look like an attribute, but XPath doesn't treat it as such.
    // Those are namespaces, and they are available through a separate axis (`namespace::`)
    // Thus we don't need to register a namespace for `xmlns` prefix

    for (prefix, uri) in namespaces.into_iter() {
        context.set_namespace(prefix.as_str(), uri.into_string()?.as_str());
    }

    // leaving this here for augmentation at some point
    // build_variables(&arguments, &mut context);
    // build_namespaces(&arguments, &mut context);
    let res = xpath.evaluate(&context, document.root());

    match res {
        Ok(sxd_xpath::Value::Boolean(b)) => Ok(Value::bool(b, call.head)),
        Ok(sxd_xpath::Value::Number(n)) => Ok(Value::float(n, call.head)),
        Ok(sxd_xpath::Value::String(s)) => Ok(Value::string(s, call.head)),
        Ok(sxd_xpath::Value::Nodeset(ns)) => {
            let mut records: Vec<Value> = vec![];
            for n in ns.document_order() {
                records.push(node_to_record(n, &node_output_options, call.head));
            }
            Ok(Value::list(records, call.head))
        }
        Err(err) => {
            Err(LabeledError::new("xpath query error").with_label(err.to_string(), call.head))
        }
    }
}

fn node_to_record(
    n: sxd_xpath::nodeset::Node<'_>,
    options: &NodeOutputOptions,
    span: Span,
) -> Value {
    use sxd_xpath::nodeset::Node;

    let mut record = record! {};

    if options.string_value {
        record.push("string_value", Value::string(n.string_value(), span));
    }

    if options.type_ {
        record.push(
            "type",
            match n {
                Node::Element(..) => Value::string("element", span),
                Node::Attribute(..) => Value::string("attribute", span),
                Node::Text(..) => Value::string("text", span),
                Node::Comment(..) => Value::string("comment", span),
                Node::ProcessingInstruction(..) => Value::string("processing_instruction", span),
                Node::Root(..) => Value::string("root", span),
                Node::Namespace(..) => Value::string("namespace", span),
            },
        );
    }

    if options.names {
        record.push(
            "local_name",
            match n.expanded_name() {
                Some(name) => Value::string(name.local_part(), span),
                None => Value::nothing(span),
            },
        );
        record.push(
            "namespace",
            match n.expanded_name() {
                Some(name) => match name.namespace_uri() {
                    Some(uri) => Value::string(uri, span),
                    None => Value::nothing(span),
                },
                None => Value::nothing(span),
            },
        );
        record.push(
            "prefixed_name",
            match n.prefixed_name() {
                Some(name) => Value::string(name, span),
                None => Value::nothing(span),
            },
        );
    }

    Value::record(record, span)
}

fn build_xpath(xpath_str: &str, span: Span) -> Result<sxd_xpath::XPath, LabeledError> {
    let factory = Factory::new();

    match factory.build(xpath_str) {
        Ok(xpath) => xpath.ok_or_else(|| {
            LabeledError::new("invalid xpath query").with_label("the query must not be empty", span)
        }),
        Err(err) => Err(LabeledError::new("invalid xpath query").with_label(err.to_string(), span)),
    }
}

struct NodeOutputOptions {
    string_value: bool,
    type_: bool,
    names: bool,
}

impl NodeOutputOptions {
    fn from_call(call: &EvaluatedCall) -> Self {
        match (
            call.has_flag("output-string-value")
                .expect("output-string-value flag"),
            call.has_flag("output-type").expect("output-type flag"),
            call.has_flag("output-names").expect("output-names flag"),
        ) {
            // no flags - old behavior - single column
            (false, false, false) => NodeOutputOptions {
                string_value: true,
                type_: false,
                names: false,
            },
            (string_value, type_, names) => NodeOutputOptions {
                string_value,
                type_,
                names,
            },
        }
    }
}

