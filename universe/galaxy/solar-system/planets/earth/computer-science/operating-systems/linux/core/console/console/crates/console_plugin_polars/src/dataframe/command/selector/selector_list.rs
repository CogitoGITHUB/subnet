use crate::{
    PolarsPlugin,
    dataframe::values::NuSelector,
    values::{CustomValueSupport, PolarsPluginType},
};
use console_plugin::{EngineInterface, EvaluatedCall, PluginCommand};
use console_protocol::{Category, Example, LabeledError, PipelineData, Signature, Type};
use polars::prelude::{DataTypeSelector, Selector};

#[derive(Clone)]
pub struct SelectorList;

impl PluginCommand for SelectorList {
    type Plugin = PolarsPlugin;

    fn name(&self) -> &str {
        "polars selector list"
    }

    fn description(&self) -> &str {
        "Select all list columns."
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .input_output_type(Type::Any, PolarsPluginType::NuSelector.into())
            .category(Category::Custom("expression".into()))
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![Example {
            example: "polars selector list",
            description: "Create a selector for list columns",
            result: None,
        }]
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["columns", "select", "list", "array"]
    }

    fn run(
        &self,
        plugin: &Self::Plugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        mut input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        let metadata = input.take_metadata();

        let selector = Selector::ByDType(DataTypeSelector::List(None));
        let shell_selector = NuSelector::from(selector);

        shell_selector
            .to_pipeline_data(plugin, engine, call.head)
            .map_err(LabeledError::from)
            .map(|pd| pd.set_metadata(metadata))
    }
}

