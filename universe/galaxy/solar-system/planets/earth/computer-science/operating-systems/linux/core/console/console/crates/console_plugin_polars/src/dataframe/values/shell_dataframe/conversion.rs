use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use indexmap::map::{Entry, IndexMap};
use polars::chunked_array::ChunkedArray;
use polars::chunked_array::builder::AnonymousOwnedListBuilder;
use polars::chunked_array::object::builder::ObjectChunkedBuilder;
use polars::datatypes::{AnyValue, PlSmallStr};
use polars::prelude::{
    CatSize, ChunkAnyValue, Column as PolarsColumn, DataFrame, DataType, DatetimeChunked,
    Float32Type, Float64Type, Int8Type, Int16Type, Int32Type, Int64Type, IntoSeries,
    ListBooleanChunkedBuilder, ListBuilderTrait, ListPrimitiveChunkedBuilder,
    ListStringChunkedBuilder, ListType, LogicalType, NamedFrom, NewChunkedArray, ObjectType,
    PolarsError, Schema, SchemaExt, Series, StructChunked, TemporalMethods, TimeUnit,
    TimeZone as PolarsTimeZone, UInt8Type, UInt16Type, UInt32Type, UInt64Type,
};

use console_protocol::shell_error::generic::GenericError;
use console_protocol::{Record, ShellError, Span, Value};
use polars::time::Duration as PolarsDuration;
use polars_arrow::Either;
use polars_arrow::array::Utf8ViewArray;

use crate::command::datetime::timezone_utc;
use crate::dataframe::values::NuSchema;

use super::{DataFrameValue, NuDataFrame};

const NANOS_PER_DAY: i64 = 86_400_000_000_000;

// The values capacity is for the size of an  vec.
// Since this is impossible to determine without traversing every value
// I just picked one. Since this is for converting back and forth
// between shell tables the values shouldn't be too extremely large for
// practical reasons (~ a few thousand rows).
const VALUES_CAPACITY: usize = 10;

macro_rules! value_to_primitive {
    ($value:ident, u8) => {
        value_to_int($value).map(|v| v as u8)
    };
    ($value:ident, u16) => {
        value_to_int($value).map(|v| v as u16)
    };
    ($value:ident, u32) => {
        value_to_int($value).map(|v| v as u32)
    };
    ($value:ident, u64) => {
        value_to_int($value).map(|v| v as u64)
    };
    ($value:ident, i8) => {
        value_to_int($value).map(|v| v as i8)
    };
    ($value:ident, i16) => {
        value_to_int($value).map(|v| v as i16)
    };
    ($value:ident, i32) => {
        value_to_int($value).map(|v| v as i32)
    };
    ($value:ident, i64) => {
        value_to_int($value)
    };
    ($value:ident, f32) => {
        $value.as_float().map(|v| v as f32)
    };
    ($value:ident, f64) => {
        $value.as_float()
    };
}

#[derive(Debug)]
pub struct Column {
    name: PlSmallStr,
    values: Vec<Value>,
}

impl Column {
    pub fn new(name: impl Into<PlSmallStr>, values: Vec<Value>) -> Self {
        Self {
            name: name.into(),
            values,
        }
    }

    pub fn new_empty(name: PlSmallStr) -> Self {
        Self {
            name,
            values: Vec::new(),
        }
    }

    pub fn name(&self) -> &PlSmallStr {
        &self.name
    }
}

impl IntoIterator for Column {
    type Item = Value;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

impl Deref for Column {
    type Target = Vec<Value>;

    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

impl DerefMut for Column {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.values
    }
}

#[derive(Debug)]
pub struct TypedColumn {
    column: Column,
    column_type: Option<DataType>,
}

impl TypedColumn {
    fn new_empty(name: PlSmallStr) -> Self {
        Self {
            column: Column::new_empty(name),
            column_type: None,
        }
    }
}

impl Deref for TypedColumn {
    type Target = Column;

    fn deref(&self) -> &Self::Target {
        &self.column
    }
}

impl DerefMut for TypedColumn {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.column
    }
}

pub type ColumnMap = IndexMap<PlSmallStr, TypedColumn>;

pub fn create_column(
    column: &PolarsColumn,
    from_row: usize,
    to_row: usize,
    span: Span,
) -> Result<Column, ShellError> {
    let series = column.as_materialized_series();
    create_column_from_series(series, from_row, to_row, span)
}

pub fn create_column_from_series(
    series: &Series,
    from_row: usize,
    to_row: usize,
    span: Span,
) -> Result<Column, ShellError> {
    let size = to_row - from_row;
    let values = series_to_values(series, Some(from_row), Some(size), span)?;
    Ok(Column::new(series.name().clone(), values))
}

// Adds a separator to the vector of values using the column names from the
// dataframe to create the Values Row
// returns true if there is an index column contained in the dataframe
pub fn add_separator(values: &mut Vec<Value>, df: &DataFrame, has_index: bool, span: Span) {
    let mut record = Record::new();

    if !has_index {
        record.push("index", Value::string("...", span));
    }

    for name in df.get_column_names() {
        // there should only be one index field
        record.push(name.as_str(), Value::string("...", span))
    }

    values.push(Value::record(record, span));
}

// Inserting the values found in a Value::List or Value::Record
pub fn insert_record(
    column_values: &mut ColumnMap,
    record: Record,
    maybe_schema: &Option<NuSchema>,
) -> Result<(), ShellError> {
    for (col, value) in record {
        insert_value(value, col.into(), column_values, maybe_schema)?;
    }

    Ok(())
}

pub fn insert_value(
    value: Value,
    key: PlSmallStr,
    column_values: &mut ColumnMap,
    maybe_schema: &Option<NuSchema>,
) -> Result<(), ShellError> {
    // If we have a schema but a key is not provided, do not create that column
    if let Some(schema) = maybe_schema
        && !schema.schema.contains(&key)
    {
        return Ok(());
    }

    let col_val = match column_values.entry(key.clone()) {
        Entry::Vacant(entry) => entry.insert(TypedColumn::new_empty(key.clone())),
        Entry::Occupied(entry) => entry.into_mut(),
    };

    // If we have a schema, use that for determining how things should be added to each column
    if let Some(schema) = maybe_schema
        && let Some(field) = schema.schema.get_field(&key)
    {
        col_val.column_type = Some(field.dtype().clone());
        col_val.values.push(value);
        return Ok(());
    }

    // If we do not have a schema, use defaults specified in `value_to_data_type`
    let current_data_type = value_to_data_type(&value);
    if col_val.column_type.is_none() {
        col_val.column_type = value_to_data_type(&value);
    } else if let Some(current_data_type) = current_data_type
        && col_val.column_type.as_ref() != Some(&current_data_type)
    {
        col_val.column_type = Some(DataType::Object("Value"));
    }
    col_val.values.push(value);

    Ok(())
}

fn value_to_data_type(value: &Value) -> Option<DataType> {
    match &value {
        Value::Int { .. } => Some(DataType::Int64),
        Value::Float { .. } => Some(DataType::Float64),
        Value::String { .. } => Some(DataType::String),
        Value::Bool { .. } => Some(DataType::Boolean),
        Value::Date { .. } => Some(DataType::Datetime(
            TimeUnit::Nanoseconds,
            Some(timezone_utc()),
        )),
        Value::Duration { .. } => Some(DataType::Duration(TimeUnit::Nanoseconds)),
        Value::Filesize { .. } => Some(DataType::Int64),
        Value::Binary { .. } => Some(DataType::Binary),
        Value::List { vals, .. } => {
            // We need to determined the type inside of the list.
            // Since Value::List does not have any kind of
            // type information, we need to look inside the list.
            // This will cause errors if lists have inconsistent types.
            // Basically, if a list column needs to be converted to dataframe,
            // needs to have consistent types.
            let list_type = vals
                .iter()
                .filter(|v| !matches!(v, Value::Nothing { .. }))
                .map(value_to_data_type)
                .nth(1)
                .flatten()
                .unwrap_or(DataType::Object("Value"));

            Some(DataType::List(Box::new(list_type)))
        }
        _ => None,
    }
}

fn typed_column_to_series(name: PlSmallStr, column: TypedColumn) -> Result<Series, ShellError> {
    let column_type = &column
        .column_type
        .clone()
        .unwrap_or(DataType::Object("Value"));
    match column_type {
        DataType::Float32 => {
            let series_values: Result<Vec<_>, _> = column
                .values
                .iter()
                .map(|v| {
                    value_to_option(v, |v| match v {
                        Value::Float { val, .. } => Ok(*val as f32),
                        Value::Int { val, .. } => Ok(*val as f32),
                        x => Err(ShellError::Generic(
                            GenericError::new_internal("Error converting to f32", "")
                                .with_help(format!("Unexpected type: {x:?}")),
                        )),
                    })
                })
                .collect();
            Ok(Series::new(name, series_values?))
        }
        DataType::Float64 => {
            let series_values: Result<Vec<_>, _> = column
                .values
                .iter()
                .map(|v| {
                    value_to_option(v, |v| match v {
                        Value::Float { val, .. } => Ok(*val),
                        Value::Int { val, .. } => Ok(*val as f64),
                        x => Err(ShellError::Generic(
                            GenericError::new_internal("Error converting to f64", "")
                                .with_help(format!("Unexpected type: {x:?}")),
                        )),
                    })
                })
                .collect();
            Ok(Series::new(name, series_values?))
        }
        DataType::Decimal(precision, scale) => {
            let series_values: Result<Vec<_>, _> = column
                .values
                .iter()
                .map(|v| {
                    value_to_option(v, |v| match v {
                        Value::Float { val, .. } => Ok(*val),
                        Value::Int { val, .. } => Ok(*val as f64),
                        x => Err(ShellError::Generic(
                            GenericError::new_internal("Error converting to decimal", "")
                                .with_help(format!("Unexpected type: {x:?}")),
                        )),
                    })
                })
                .collect();
            Series::new(name, series_values?)
                .cast_with_options(&DataType::Decimal(*precision, *scale), Default::default())
                .map_err(|e| {
                    ShellError::Generic(
                        GenericError::new_internal("Error parsing decimal", "")
                            .with_help(e.to_string()),
                    )
                })
        }
        DataType::UInt8 => {
            let series_values: Result<Vec<_>, _> = column
                .values
                .iter()
                .map(|v| value_to_option(v, |v| value_to_int(v).map(|v| v as u8)))
                .collect();
            Ok(Series::new(name, series_values?))
        }
        DataType::UInt16 => {
            let series_values: Result<Vec<_>, _> = column
                .values
                .iter()
                .map(|v| value_to_option(v, |v| value_to_int(v).map(|v| v as u16)))
                .collect();
            Ok(Series::new(name, series_values?))
        }
        DataType::UInt32 => {
            let series_values: Result<Vec<_>, _> = column
                .values
                .iter()
                .map(|v| value_to_option(v, |v| value_to_int(v).map(|v| v as u32)))
                .collect();
            Ok(Series::new(name, series_values?))
        }
        DataType::UInt64 => {
            let series_values: Result<Vec<_>, _> = column
                .values
                .iter()
                .map(|v| value_to_option(v, |v| value_to_int(v).map(|v| v as u64)))
                .collect();
            Ok(Series::new(name, series_values?))
        }
        DataType::Int8 => {
            let series_values: Result<Vec<_>, _> = column
                .values
                .iter()
                .map(|v| value_to_option(v, |v| value_to_int(v).map(|v| v as i8)))
                .collect();
            Ok(Series::new(name, series_values?))
        }
        DataType::Int16 => {
            let series_values: Result<Vec<_>, _> = column
                .values
                .iter()
                .map(|v| value_to_option(v, |v| value_to_int(v).map(|v| v as i16)))
                .collect();
            Ok(Series::new(name, series_values?))
        }
        DataType::Int32 => {
            let series_values: Result<Vec<_>, _> = column
                .values
                .iter()
                .map(|v| value_to_option(v, |v| value_to_int(v).map(|v| v as i32)))
                .collect();
            Ok(Series::new(name, series_values?))
        }
        DataType::Int64 => {
            let series_values: Result<Vec<_>, _> = column
                .values
                .iter()
                .map(|v| value_to_option(v, value_to_int))
                .collect();
            Ok(Series::new(name, series_values?))
        }
        DataType::Boolean => {
            let series_values: Result<Vec<_>, _> = column
                .values
                .iter()
                .map(|v| value_to_option(v, |v| v.as_bool()))
                .collect();
            Ok(Series::new(name, series_values?))
        }
        DataType::String => {
            let series_values: Result<Vec<_>, _> = column
                .values
                .iter()
                .map(|v| value_to_option(v, |v| v.coerce_string()))
                .collect();
            Ok(Series::new(name, series_values?))
        }
        DataType::Binary | DataType::BinaryOffset => {
            let series_values: Result<Vec<_>, _> =
                column.values.iter().map(|v| v.coerce_binary()).collect();
            Ok(Series::new(name, series_values?))
        }
        DataType::Object(_) => value_to_series(name, &column.values),
        DataType::Duration(time_unit) => {
            let series_values: Result<Vec<_>, _> = column
                .values
                .iter()
                .map(|v| {
                    value_to_option(v, |v| {
                        v.as_duration().map(|v| nanos_to_timeunit(v, *time_unit))
                    }?)
                })
                .collect();
            Ok(Series::new(name, series_values?))
        }
        DataType::List(list_type) => {
            match input_type_list_to_series(&name, list_type.as_ref(), &column.values) {
                Ok(series) => Ok(series),
                Err(_) => {
                    // An error case will occur when there are lists of mixed types.
                    // If this happens, fallback to object list
                    input_type_list_to_series(&name, &DataType::Object("unknown"), &column.values)
                }
            }
        }
        DataType::Date => {
            let it = column
                .values
                .iter()
                .map(|v| match &v {
                    Value::Date { val, .. } => {
                        Ok(Some(val.timestamp_nanos_opt().unwrap_or_default()))
                    }

                    Value::String { val, .. } => {
                        let expected_format = "%Y-%m-%d";
                        let nanos = NaiveDate::parse_from_str(val, expected_format)
                            .map_err(|e| {
                                ShellError::Generic(
                                    GenericError::new_internal(
                                        format!("Error parsing date from string: {e}"),
                                        "",
                                    )
                                    .with_help(format!(
                                        "Expected format {expected_format}. If you need to parse with another format, please set the schema to `str` and parse with `polars as-date <format>`."
                                    )),
                                )
                            })?
                            .and_hms_nano_opt(0, 0, 0, 0)
                            .and_then(|dt| dt.and_utc().timestamp_nanos_opt());
                        Ok(nanos)
                    }

                    _ => Ok(None),
                })
                .collect::<Result<Vec<_>, ShellError>>()?;

            ChunkedArray::<Int64Type>::from_iter_options(name, it.into_iter())
                .into_datetime(TimeUnit::Nanoseconds, None)
                .cast_with_options(&DataType::Date, Default::default())
                .map_err(|e| {
                    ShellError::Generic(
                        GenericError::new_internal("Error parsing date", "")
                            .with_help(e.to_string()),
                    )
                })
        }
        DataType::Datetime(tu, maybe_tz) => {
            let dates = column
                .values
                .iter()
                .map(|v| {
                    match (maybe_tz, &v) {
                        (Some(tz), Value::Date { val, .. }) => {
                            // If there is a timezone specified, make sure
                            // the value is converted to it
                            tz.parse::<Tz>()
                                .map(|tz| val.with_timezone(&tz))
                                .map_err(|e| {
                                    ShellError::Generic(
                                        GenericError::new_internal("Error parsing timezone", "")
                                            .with_help(e.to_string()),
                                    )
                                })?
                                .timestamp_nanos_opt()
                                .map(|nanos| nanos_to_timeunit(nanos, *tu))
                                .transpose()
                        }
                        (None, Value::Date { val, .. }) => val
                            .timestamp_nanos_opt()
                            .map(|nanos| nanos_to_timeunit(nanos, *tu))
                            .transpose(),

                        (Some(_), Value::String { val, .. }) => {
                            // because we're converting to the number of nano seconds since epoch, the timezone is irrelevant
                            let expected_format = "%Y-%m-%d %H:%M:%S%:z";
                            DateTime::parse_from_str(val, expected_format)
                                .map_err(|e| {
                                    ShellError::Generic(
                                        GenericError::new_internal(
                                            format!("Error parsing datetime from string: {e}"),
                                            "",
                                        )
                                        .with_help(format!(
                                            "Expected format {expected_format}. If you need to parse with another format, please set the schema to `str` and parse with `polars as-datetime <format>`."
                                        )),
                                    )
                                })?
                                .timestamp_nanos_opt()
                                .map(|nanos| nanos_to_timeunit(nanos, *tu))
                                .transpose()
                        }

                        (None, Value::String { val, .. }) => {
                            let expected_format = "%Y-%m-%d %H:%M:%S";

                            NaiveDateTime::parse_from_str(val, expected_format)
                                .map_err(|e| {
                                    ShellError::Generic(
                                        GenericError::new_internal(
                                            format!("Error parsing datetime from string: {e}"),
                                            "",
                                        )
                                        .with_help(format!(
                                            "Expected format {expected_format}. If you need to parse with another format, please set the schema to `str` and parse with `polars as-datetime <format>`."
                                        )),
                                    )
                                })?
                                .and_utc()
                                .timestamp_nanos_opt()
                                .map(|nanos| nanos_to_timeunit(nanos, *tu))
                                .transpose()
                        }

                        _ => Ok(None),
                    }
                })
                .collect::<Result<Vec<Option<i64>>, ShellError>>()?;

            let res: DatetimeChunked =
                ChunkedArray::<Int64Type>::from_iter_options(name, dates.into_iter())
                    .into_datetime(*tu, maybe_tz.clone());

            Ok(res.into_series())
        }
        DataType::Struct(fields) => {
            let schema = Some(NuSchema::new(Arc::new(Schema::from_iter(fields.clone()))));
            // let mut structs: Vec<Series> = Vec::new();
            let mut structs: HashMap<PlSmallStr, Series> = HashMap::new();

            for v in column.values.iter() {
                let mut column_values: ColumnMap = IndexMap::new();
                let record = v.as_record()?;
                insert_record(&mut column_values, record.clone(), &schema)?;
                let df = from_parsed_columns(column_values)?;
                for name in df.df.get_column_names() {
                    let series = df
                        .df
                        .column(name)
                        .map_err(|e| {
                            ShellError::Generic(GenericError::new_internal(
                                format!(
                                    "Error creating struct, could not get column name {name}: {e}"
                                ),
                                "",
                            ))
                        })?
                        .as_materialized_series();

                    if let Some(v) = structs.get_mut(name) {
                        let _ = v.append(series).map_err(|e| {
                            ShellError::Generic(GenericError::new_internal(
                                format!(
                                    "Error creating struct, could not append to series for col {name}: {e}"
                                ),
                                "",
                            ))
                        })?;
                    } else {
                        structs.insert(name.clone(), series.to_owned());
                    }
                }
            }

            let structs: Vec<Series> = structs.into_values().collect();

            let chunked =
                StructChunked::from_series(column.name().to_owned(), structs.len(), structs.iter())
                    .map_err(|e| {
                        ShellError::Generic(GenericError::new_internal(
                            format!("Error creating struct: {e}"),
                            "",
                        ))
                    })?;
            Ok(chunked.into_series())
        }
        _ => Err(ShellError::Generic(GenericError::new_internal(
            format!("Error creating dataframe: Unsupported type: {column_type:?}"),
            "",
        ))),
    }
}

// The ColumnMap has the parsed data from the StreamInput
// This data can be used to create a Series object that can initialize
// the dataframe based on the type of data that is found
pub fn from_parsed_columns(column_values: ColumnMap) -> Result<NuDataFrame, ShellError> {
    let mut df_columns: Vec<PolarsColumn> = Vec::new();
    for (name, column) in column_values {
        let series = typed_column_to_series(name, column)?;
        df_columns.push(series.into());
    }

    DataFrame::new_infer_height(df_columns)
        .map(|df| NuDataFrame::new(false, df))
        .map_err(|e| {
            ShellError::Generic(GenericError::new_internal(
                "Error creating dataframe",
                e.to_string(),
            ))
        })
}

pub fn value_to_series(name: PlSmallStr, values: &[Value]) -> Result<Series, ShellError> {
    // try to get the type, if the list is inconsistent, fallback to object type
    if let Some(data_type) = values.first().and_then(value_to_data_type) {
        let column = TypedColumn {
            column: Column::new(name.clone(), values.to_owned()),
            column_type: Some(data_type.clone()),
        };

        typed_column_to_series(name.clone(), column)
            .or_else(|e| {
                eprintln!("Error converting list to series with type {data_type:?}: {e}. Falling back to object type.");
                value_to_object_series(name, values)
            })
    } else {
        value_to_object_series(name, values)
    }
}

fn value_to_object_series(name: PlSmallStr, values: &[Value]) -> Result<Series, ShellError> {
    let mut builder = ObjectChunkedBuilder::<DataFrameValue>::new(name, values.len());

    for v in values {
        builder.append_value(DataFrameValue::new(v.clone()));
    }

    let res = builder.finish();
    Ok(res.into_series())
}

fn input_type_list_to_series(
    name: &PlSmallStr,
    data_type: &DataType,
    values: &[Value],
) -> Result<Series, ShellError> {
    let inconsistent_error = |_| {
        ShellError::Generic(GenericError::new_internal(
            format!(
                "column {name} contains a list with inconsistent types: Expecting: {data_type:?}"
            ),
            "",
        ))
    };

    macro_rules! primitive_list_series {
        ($list_type:ty, $vec_type:tt) => {{
            let mut builder = ListPrimitiveChunkedBuilder::<$list_type>::new(
                name.clone(),
                values.len(),
                VALUES_CAPACITY,
                data_type.clone(),
            );

            for v in values {
                let value_list = v
                    .as_list()?
                    .iter()
                    .map(|v| value_to_primitive!(v, $vec_type))
                    .collect::<Result<Vec<$vec_type>, _>>()
                    .map_err(inconsistent_error)?;
                builder.append_values_iter(value_list.iter().copied());
            }
            let res = builder.finish();
            Ok(res.into_series())
        }};
    }

    match *data_type {
        // list of boolean values
        DataType::Boolean => {
            let mut builder =
                ListBooleanChunkedBuilder::new(name.clone(), values.len(), VALUES_CAPACITY);
            for v in values {
                let value_list = v
                    .as_list()?
                    .iter()
                    .map(|v| v.as_bool())
                    .collect::<Result<Vec<bool>, _>>()
                    .map_err(inconsistent_error)?;
                builder.append_iter(value_list.iter().map(|v| Some(*v)));
            }
            let res = builder.finish();
            Ok(res.into_series())
        }
        DataType::Duration(_) => primitive_list_series!(Int64Type, i64),
        DataType::UInt8 => primitive_list_series!(UInt8Type, u8),
        DataType::UInt16 => primitive_list_series!(UInt16Type, u16),
        DataType::UInt32 => primitive_list_series!(UInt32Type, u32),
        DataType::UInt64 => primitive_list_series!(UInt64Type, u64),
        DataType::Int8 => primitive_list_series!(Int8Type, i8),
        DataType::Int16 => primitive_list_series!(Int16Type, i16),
        DataType::Int32 => primitive_list_series!(Int32Type, i32),
        DataType::Int64 => primitive_list_series!(Int64Type, i64),
        DataType::Float32 => primitive_list_series!(Float32Type, f32),
        DataType::Float64 => primitive_list_series!(Float64Type, f64),
        DataType::String => {
            let mut builder =
                ListStringChunkedBuilder::new(name.clone(), values.len(), VALUES_CAPACITY);
            for v in values {
                let value_list = v
                    .as_list()?
                    .iter()
                    .map(|v| v.coerce_string())
                    .collect::<Result<Vec<String>, _>>()
                    .map_err(inconsistent_error)?;
                builder.append_values_iter(value_list.iter().map(AsRef::as_ref));
            }
            let res = builder.finish();
            Ok(res.into_series())
        }
        DataType::Date => {
            let mut builder = AnonymousOwnedListBuilder::new(
                name.clone(),
                values.len(),
                Some(DataType::Datetime(TimeUnit::Nanoseconds, None)),
            );
            for (i, v) in values.iter().enumerate() {
                let list_name = i.to_string();

                let it = v.as_list()?.iter().map(|v| {
                    if let Value::Date { val, .. } = &v {
                        Some(val.timestamp_nanos_opt().unwrap_or_default())
                    } else {
                        None
                    }
                });
                let dt_chunked = ChunkedArray::<Int64Type>::from_iter_options(list_name.into(), it)
                    .into_datetime(TimeUnit::Nanoseconds, None);

                builder
                    .append_series(&dt_chunked.into_series())
                    .map_err(|e| {
                        ShellError::Generic(
                            GenericError::new_internal("Error appending to series", "")
                                .with_help(e.to_string()),
                        )
                    })?
            }
            let res = builder.finish();
            Ok(res.into_series())
        }
        DataType::List(ref sub_list_type) => {
            Ok(input_type_list_to_series(name, sub_list_type, values)?)
        }
        // treat everything else as an object
        _ => Ok(value_to_series(name.clone(), values)?),
    }
}

fn series_to_values(
    series: &Series,
    maybe_from_row: Option<usize>,
    maybe_size: Option<usize>,
    span: Span,
) -> Result<Vec<Value>, ShellError> {
    match series.dtype() {
        DataType::Null => {
            if let Some(size) = maybe_size {
                Ok(vec![Value::nothing(span); size])
            } else {
                Ok(vec![Value::nothing(span); series.len()])
            }
        }
        DataType::UInt8 => {
            let casted = series.u8().map_err(|e| {
                ShellError::Generic(
                    GenericError::new_internal("Error casting column to u8", "")
                        .with_help(e.to_string()),
                )
            })?;

            let it = casted.iter();
            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(a) => Value::int(a as i64, span),
                None => Value::nothing(span),
            })
            .collect::<Vec<Value>>();

            Ok(values)
        }
        DataType::UInt16 => {
            let casted = series.u16().map_err(|e| {
                ShellError::Generic(
                    GenericError::new_internal("Error casting column to u16", "")
                        .with_help(e.to_string()),
                )
            })?;

            let it = casted.iter();
            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(a) => Value::int(a as i64, span),
                None => Value::nothing(span),
            })
            .collect::<Vec<Value>>();

            Ok(values)
        }
        DataType::UInt32 => {
            let casted = series.u32().map_err(|e| {
                ShellError::Generic(
                    GenericError::new_internal("Error casting column to u32", "")
                        .with_help(e.to_string()),
                )
            })?;

            let it = casted.iter();
            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(a) => Value::int(a as i64, span),
                None => Value::nothing(span),
            })
            .collect::<Vec<Value>>();

            Ok(values)
        }
        DataType::UInt64 => {
            let casted = series.u64().map_err(|e| {
                ShellError::Generic(
                    GenericError::new_internal("Error casting column to u64", "")
                        .with_help(e.to_string()),
                )
            })?;

            let it = casted.iter();
            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(a) => Value::int(a as i64, span),
                None => Value::nothing(span),
            })
            .collect::<Vec<Value>>();

            Ok(values)
        }
        DataType::Int8 => {
            let casted = series.i8().map_err(|e| {
                ShellError::Generic(
                    GenericError::new_internal("Error casting column to i8", "")
                        .with_help(e.to_string()),
                )
            })?;

            let it = casted.iter();
            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(a) => Value::int(a as i64, span),
                None => Value::nothing(span),
            })
            .collect::<Vec<Value>>();

            Ok(values)
        }
        DataType::Int16 => {
            let casted = series.i16().map_err(|e| {
                ShellError::Generic(
                    GenericError::new_internal("Error casting column to i16", "")
                        .with_help(e.to_string()),
                )
            })?;

            let it = casted.iter();
            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(a) => Value::int(a as i64, span),
                None => Value::nothing(span),
            })
            .collect::<Vec<Value>>();

            Ok(values)
        }
        DataType::Int32 => {
            let casted = series.i32().map_err(|e| {
                ShellError::Generic(
                    GenericError::new_internal("Error casting column to i32", "")
                        .with_help(e.to_string()),
                )
            })?;

            let it = casted.iter();
            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(a) => Value::int(a as i64, span),
                None => Value::nothing(span),
            })
            .collect::<Vec<Value>>();

            Ok(values)
        }
        DataType::Int64 => {
            let casted = series.i64().map_err(|e| {
                ShellError::Generic(
                    GenericError::new_internal("Error casting column to i64", "")
                        .with_help(e.to_string()),
                )
            })?;

            let it = casted.iter();
            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(a) => Value::int(a, span),
                None => Value::nothing(span),
            })
            .collect::<Vec<Value>>();

            Ok(values)
        }
        DataType::Float32 => {
            let casted = series.f32().map_err(|e| {
                ShellError::Generic(
                    GenericError::new_internal("Error casting column to f32", "")
                        .with_help(e.to_string()),
                )
            })?;

            let it = casted.iter();
            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(a) => Value::float(a as f64, span),
                None => Value::nothing(span),
            })
            .collect::<Vec<Value>>();

            Ok(values)
        }
        DataType::Float64 => {
            let casted = series.f64().map_err(|e| {
                ShellError::Generic(
                    GenericError::new_internal("Error casting column to f64", "")
                        .with_help(e.to_string()),
                )
            })?;

            let it = casted.iter();
            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(a) => Value::float(a, span),
                None => Value::nothing(span),
            })
            .collect::<Vec<Value>>();

            Ok(values)
        }
        DataType::Boolean => {
            let casted = series.bool().map_err(|e| {
                ShellError::Generic(
                    GenericError::new_internal("Error casting column to bool", "")
                        .with_help(e.to_string()),
                )
            })?;

            let it = casted.iter();
            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(a) => Value::bool(a, span),
                None => Value::nothing(span),
            })
            .collect::<Vec<Value>>();

            Ok(values)
        }
        DataType::String => {
            let casted = series.str().map_err(|e| {
                ShellError::Generic(
                    GenericError::new_internal("Error casting column to string", "")
                        .with_help(e.to_string()),
                )
            })?;

            let it = casted.iter();
            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(a) => Value::string(a.to_string(), span),
                None => Value::nothing(span),
            })
            .collect::<Vec<Value>>();

            Ok(values)
        }
        DataType::Binary => {
            let make_err = |e: PolarsError| {
                ShellError::Generic(
                    GenericError::new_internal("Error casting column to binary", "")
                        .with_help(e.to_string()),
                )
            };

            let it = series.binary().map_err(make_err)?.iter();

            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(b) => Value::binary(b, span),
                None => Value::nothing(span),
            })
            .collect::<Vec<Value>>();

            Ok(values)
        }
        DataType::BinaryOffset => {
            let make_err = |e: PolarsError| {
                ShellError::Generic(
                    GenericError::new_internal("Error casting column to binary offset", "")
                        .with_help(e.to_string()),
                )
            };

            let it = series.binary_offset().map_err(make_err)?.iter();

            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(b) => Value::binary(b, span),
                None => Value::nothing(span),
            })
            .collect::<Vec<Value>>();

            Ok(values)
        }
        DataType::Object(x) => {
            let casted = series
                .as_any()
                .downcast_ref::<ChunkedArray<ObjectType<DataFrameValue>>>();

            match casted {
                None => Err(ShellError::Generic(
                    GenericError::new_internal("Error casting object from series", "")
                        .with_help(format!("Object not supported for conversion: {x}")),
                )),
                Some(ca) => {
                    let it = ca.iter();
                    let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row)
                    {
                        Either::Left(it.skip(from_row).take(size))
                    } else {
                        Either::Right(it)
                    }
                    .map(|v| match v {
                        Some(a) => a.get_value(),
                        None => Value::nothing(span),
                    })
                    .collect::<Vec<Value>>();

                    Ok(values)
                }
            }
        }
        DataType::List(x) => {
            let casted = series.as_any().downcast_ref::<ChunkedArray<ListType>>();
            match casted {
                None => Err(ShellError::Generic(
                    GenericError::new_internal("Error casting list from series", "")
                        .with_help(format!("List not supported for conversion: {x}")),
                )),
                Some(ca) => {
                    let it = ca.iter();
                    if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                        Either::Left(it.skip(from_row).take(size))
                    } else {
                        Either::Right(it)
                    }
                    .map(|ca| {
                        let sublist: Vec<Value> = if let Some(arr) = ca {
                            let dt = DataType::from_arrow_dtype(arr.dtype());
                            let s =
                                Series::from_chunk_and_dtype("".into(), arr, &dt).map_err(|e| {
                                    ShellError::Generic(
                                        GenericError::new_internal(
                                            "Error creating series from list values",
                                            "",
                                        )
                                        .with_help(e.to_string()),
                                    )
                                })?;
                            series_to_values(&s, None, None, span)?
                        } else {
                            // empty item
                            vec![]
                        };
                        Ok(Value::list(sublist, span))
                    })
                    .collect::<Result<Vec<Value>, ShellError>>()
                }
            }
        }
        DataType::Date => {
            let casted = series
                .date()
                .map_err(|e| {
                    ShellError::Generic(
                        GenericError::new_internal("Error casting column to date", "")
                            .with_help(e.to_string()),
                    )
                })?
                .clone()
                .into_physical();

            let it = casted.iter();
            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(a) => {
                    let nanos = nanos_per_day(a);
                    let datetime = datetime_from_epoch_nanos(nanos, None, span)?;
                    Ok(Value::date(datetime, span))
                }
                None => Ok(Value::nothing(span)),
            })
            .collect::<Result<Vec<Value>, ShellError>>()?;
            Ok(values)
        }
        DataType::Datetime(time_unit, tz) => {
            let casted = series
                .datetime()
                .map_err(|e| {
                    ShellError::Generic(
                        GenericError::new_internal("Error casting column to datetime", "")
                            .with_help(e.to_string()),
                    )
                })?
                .clone()
                .into_physical();

            let it = casted.iter();
            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(a) => {
                    // elapsed time in nano/micro/milliseconds since 1970-01-01
                    let nanos = nanos_from_timeunit(a, *time_unit)?;
                    let datetime = datetime_from_epoch_nanos(nanos, tz.as_ref(), span)?;
                    Ok(Value::date(datetime, span))
                }
                None => Ok(Value::nothing(span)),
            })
            .collect::<Result<Vec<Value>, ShellError>>()?;
            Ok(values)
        }
        DataType::Duration(time_unit) => {
            let casted = series
                .duration()
                .map_err(|e| {
                    ShellError::Generic(
                        GenericError::new_internal("Error casting column to duration", "")
                            .with_help(e.to_string()),
                    )
                })?
                .clone()
                .into_physical();

            let it = casted.iter();
            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(a) => {
                    let nanos = nanos_from_timeunit(a, *time_unit)?;
                    Ok(Value::duration(nanos, span))
                }
                None => Ok(Value::nothing(span)),
            })
            .collect::<Result<Vec<Value>, ShellError>>()?;
            Ok(values)
        }
        DataType::Struct(_) => {
            let casted = series.struct_().map_err(|e| {
                ShellError::Generic(
                    GenericError::new_internal("Error casting column to struct", "")
                        .with_help(e.to_string()),
                )
            })?;

            let range = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                from_row..(from_row + size)
            } else {
                0..casted.len()
            };

            let mut values = Vec::with_capacity(casted.len());

            for i in range {
                let val = casted.get_any_value(i).map_err(|e| {
                    ShellError::Generic(GenericError::new_internal(
                        format!("Could not get struct value for index {i} - {e}"),
                        "",
                    ))
                })?;
                values.push(any_value_to_value(&val, span)?)
            }

            Ok(values)
        }
        DataType::Time => {
            let casted = series.timestamp(TimeUnit::Nanoseconds).map_err(|e| {
                ShellError::Generic(
                    GenericError::new_internal("Error casting column to time", "")
                        .with_help(e.to_string()),
                )
            })?;

            let it = casted.iter();
            let values = if let (Some(size), Some(from_row)) = (maybe_size, maybe_from_row) {
                Either::Left(it.skip(from_row).take(size))
            } else {
                Either::Right(it)
            }
            .map(|v| match v {
                Some(nanoseconds) => Value::duration(nanoseconds, span),
                None => Value::nothing(span),
            })
            .collect::<Vec<Value>>();

            Ok(values)
        }
        DataType::Decimal(_precision, _scale) => {
            let casted = series.cast(&DataType::Float64).map_err(|e| {
                ShellError::Generic(
                    GenericError::new_internal("Errors casting decimal column to float", "")
                        .with_help(e.to_string()),
                )
            })?;
            series_to_values(&casted, maybe_from_row, maybe_size, span)
        }
        DataType::Categorical(categories, _categorical_ordering) => Ok(utf8_view_array_to_value(
            categories.freeze().categories(),
            span,
        )),
        DataType::Enum(frozen_categories, _categorical_ordering) => Ok(utf8_view_array_to_value(
            frozen_categories.categories(),
            span,
        )),
        e => Err(ShellError::Generic(
            GenericError::new_internal("Error creating Dataframe", "")
                .with_help(format!("Value not supported in shell: {e:?}")),
        )),
    }
}

fn any_value_to_value(any_value: &AnyValue, span: Span) -> Result<Value, ShellError> {
    match any_value {
        AnyValue::Null => Ok(Value::nothing(span)),
        AnyValue::Boolean(b) => Ok(Value::bool(*b, span)),
        AnyValue::String(s) => Ok(Value::string(s.to_string(), span)),
        AnyValue::UInt8(i) => Ok(Value::int(*i as i64, span)),
        AnyValue::UInt16(i) => Ok(Value::int(*i as i64, span)),
        AnyValue::UInt32(i) => Ok(Value::int(*i as i64, span)),
        AnyValue::UInt64(i) => Ok(Value::int(*i as i64, span)),
        AnyValue::Int8(i) => Ok(Value::int(*i as i64, span)),
        AnyValue::Int16(i) => Ok(Value::int(*i as i64, span)),
        AnyValue::Int32(i) => Ok(Value::int(*i as i64, span)),
        AnyValue::Int64(i) => Ok(Value::int(*i, span)),
        AnyValue::Float32(f) => Ok(Value::float(*f as f64, span)),
        AnyValue::Float64(f) => Ok(Value::float(*f, span)),
        AnyValue::Date(d) => {
            let nanos = nanos_per_day(*d);
            datetime_from_epoch_nanos(nanos, None, span).map(|datetime| Value::date(datetime, span))
        }
        AnyValue::Datetime(a, time_unit, tz) => {
            let nanos = nanos_from_timeunit(*a, *time_unit)?;
            datetime_from_epoch_nanos(nanos, tz.cloned().as_ref(), span)
                .map(|datetime| Value::date(datetime, span))
        }
        AnyValue::DatetimeOwned(a, time_unit, tz) => {
            let nanos = nanos_from_timeunit(*a, *time_unit)?;
            datetime_from_epoch_nanos(nanos, tz.as_ref().map(|tz| tz.as_ref()), span)
                .map(|datetime| Value::date(datetime, span))
        }
        AnyValue::Duration(a, time_unit) => {
            let nanos = match time_unit {
                TimeUnit::Nanoseconds => *a,
                TimeUnit::Microseconds => *a * 1_000,
                TimeUnit::Milliseconds => *a * 1_000_000,
            };
            Ok(Value::duration(nanos, span))
        }
        // AnyValue::Time represents the current time since midnight.
        // Unfortunately, there is no timezone related information.
        // Given this, calculate the current date from UTC and add the time.
        AnyValue::Time(nanos) => time_from_midnight(*nanos, span),
        AnyValue::List(series) => {
            series_to_values(series, None, None, span).map(|values| Value::list(values, span))
        }
        AnyValue::Struct(_idx, _struct_array, _s_fields) => {
            // This should convert to a StructOwned object.
            let static_value = any_value.clone().into_static();
            any_value_to_value(&static_value, span)
        }
        AnyValue::StructOwned(struct_tuple) => {
            let record = struct_tuple
                .1
                .iter()
                .zip(&struct_tuple.0)
                .map(|(field, val)| {
                    any_value_to_value(val, span).map(|val| (field.name.to_string(), val))
                })
                .collect::<Result<_, _>>()?;

            Ok(Value::record(record, span))
        }
        AnyValue::StringOwned(s) => Ok(Value::string(s.to_string(), span)),
        AnyValue::Binary(bytes) => Ok(Value::binary(*bytes, span)),
        AnyValue::BinaryOwned(bytes) => Ok(Value::binary(bytes.to_owned(), span)),
        AnyValue::Categorical(_cat_size, categorical_mapping)
        | AnyValue::Enum(_cat_size, categorical_mapping) => {
            let n = categorical_mapping.num_cats_upper_bound();
            let mut values: Vec<Value> = Vec::with_capacity(n);
            for i in 0..n {
                let s = categorical_mapping
                    .cat_to_str(i as CatSize)
                    .unwrap_or_default();
                values.push(Value::string(s, span));
            }

            Ok(Value::list(values, span))
        }
        AnyValue::CategoricalOwned(_cat_size, categorical_mapping)
        | AnyValue::EnumOwned(_cat_size, categorical_mapping) => {
            let n = categorical_mapping.num_cats_upper_bound();
            let mut values: Vec<Value> = Vec::with_capacity(n);
            for i in 0..n {
                let s = categorical_mapping
                    .cat_to_str(i as CatSize)
                    .unwrap_or_default();
                values.push(Value::string(s, span));
            }

            Ok(Value::list(values, span))
        }
        e => Err(ShellError::Generic(
            GenericError::new("Error creating Value", "", span)
                .with_help(format!("Value not supported in shell: {e:?}")),
        )),
    }
}

fn nanos_per_day(days: i32) -> i64 {
    days as i64 * NANOS_PER_DAY
}

fn nanos_from_timeunit(a: i64, time_unit: TimeUnit) -> Result<i64, ShellError> {
    a.checked_mul(match time_unit {
        TimeUnit::Microseconds => 1_000, // Convert microseconds to nanoseconds
        TimeUnit::Milliseconds => 1_000_000, // Convert milliseconds to nanoseconds
        TimeUnit::Nanoseconds => 1,      // Already in nanoseconds
    })
    .ok_or_else(|| {
        ShellError::Generic(GenericError::new_internal(
            format!("Converting from {time_unit} to nanoseconds caused an overflow"),
            "",
        ))
    })
}

fn nanos_to_timeunit(a: i64, time_unit: TimeUnit) -> Result<i64, ShellError> {
    // integer division (rounds to 0)
    a.checked_div(match time_unit {
        TimeUnit::Microseconds => 1_000i64, // Convert microseconds to nanoseconds
        TimeUnit::Milliseconds => 1_000_000i64, // Convert milliseconds to nanoseconds
        TimeUnit::Nanoseconds => 1i64,      // Already in nanoseconds
    })
    .ok_or_else(|| {
        ShellError::Generic(GenericError::new_internal(
            format!("Converting from nanoseconds to {time_unit} caused an overflow"),
            "",
        ))
    })
}

fn datetime_from_epoch_nanos(
    nanos: i64,
    timezone: Option<&PolarsTimeZone>,
    span: Span,
) -> Result<DateTime<FixedOffset>, ShellError> {
    let tz: Tz = if let Some(polars_tz) = timezone {
        polars_tz.parse::<Tz>().map_err(|_| {
            ShellError::Generic(GenericError::new(
                format!("Could not parse polars timezone: {polars_tz}"),
                "",
                span,
            ))
        })?
    } else {
        Tz::UTC
    };

    Ok(tz.timestamp_nanos(nanos).fixed_offset())
}

fn time_from_midnight(nanos: i64, span: Span) -> Result<Value, ShellError> {
    let today = Utc::now().date_naive();
    NaiveTime::from_hms_opt(0, 0, 0) // midnight
        .map(|time| time + Duration::nanoseconds(nanos)) // current time
        .map(|time| today.and_time(time)) // current date and time
        .and_then(|datetime| {
            FixedOffset::east_opt(0) // utc
                .map(|offset| {
                    DateTime::<FixedOffset>::from_naive_utc_and_offset(datetime, offset)
                })
        })
        .map(|datetime| Value::date(datetime, span)) // current date and time
        .ok_or(ShellError::CantConvert {
            to_type: "datetime".to_string(),
            from_type: "polars time".to_string(),
            span,
            help: Some("Could not convert polars time of {nanos} to datetime".to_string()),
        })
}

// this takes into non-int types that we should represent as int like filesize
fn value_to_int(value: &Value) -> Result<i64, ShellError> {
    match value {
        Value::Int { val, .. } => Ok(*val),
        Value::Filesize { val, .. } => Ok((*val).into()),
        _ => Err(ShellError::CantConvert {
            to_type: "int".into(),
            from_type: value.get_type().to_string(),
            span: value.span(),
            help: None,
        }),
    }
}

fn value_to_option<T, F>(value: &Value, func: F) -> Result<Option<T>, ShellError>
where
    F: FnOnce(&Value) -> Result<T, ShellError>,
{
    if value.is_nothing() {
        Ok(None)
    } else {
        func(value).map(|v| Some(v))
    }
}

fn utf8_view_array_to_value(array: &Utf8ViewArray, span: Span) -> Vec<Value> {
    array
        .iter()
        .map(|x| match x {
            Some(s) => Value::string(s.to_string(), span),
            None => Value::nothing(span),
        })
        .collect::<Vec<Value>>()
}

pub fn convert_duration(v: Value) -> Result<PolarsDuration, ShellError> {
    match v {
        Value::Duration { val, .. } => Ok(PolarsDuration::new(val)),
        Value::Int { val, .. } => Ok(PolarsDuration::new(val)),
        Value::String { ref val, .. } => {
            PolarsDuration::try_parse(val).map_err(|e| ShellError::CantConvert {
                to_type: "duration".into(),
                from_type: "string".into(),
                span: v.span(),
                help: Some(format!("Could not parse string as duration: {e}")),
            })
        }
        _ => Err(ShellError::CantConvert {
            to_type: "duration".into(),
            from_type: v.get_type().to_string(),
            span: v.span(),
            help: None,
        }),
    }
}

