pub fn did_you_mean<'a, 'b, I, S>(possibilities: I, input: &'b str) -> Option<String>
where
    I: IntoIterator<Item = &'a S>,
    S: AsRef<str> + 'a + ?Sized,
{
    let possibilities: Vec<&str> = possibilities.into_iter().map(|s| s.as_ref()).collect();
    let suggestion =
        crate::lev_distance::find_best_match_for_name_with_substrings(&possibilities, input, None)
            .map(|s| s.to_string());
    if let Some(suggestion) = &suggestion
        && suggestion.len() == 1
        && !suggestion.eq_ignore_ascii_case(input)
    {
        return None;
    }
    suggestion
}

