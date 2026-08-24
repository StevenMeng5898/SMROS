pub(crate) fn sorted_path_search<T, F>(
    records: &[T],
    path: &str,
    path_of: F,
) -> Result<usize, usize>
where
    F: for<'a> Fn(&'a T) -> &'a str,
{
    records.binary_search_by(|record| path_of(record).cmp(path))
}
