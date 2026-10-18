use anyhow::Result;
/// Host-only launch injection. Environment values never implement Debug or Serialize.
#[derive(Default)]
pub struct LaunchEnvironment(pub(crate) Vec<(String, String)>);
impl std::fmt::Debug for LaunchEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LaunchEnvironment([redacted])")
    }
}
impl LaunchEnvironment {
    pub fn insert(&mut self, name: String, value: String) -> Result<()> {
        anyhow::ensure!(
            !name.is_empty() && !name.contains(['=', '\0']) && !value.contains('\0'),
            "invalid launch environment entry"
        );
        anyhow::ensure!(
            self.0.len() < 256
                && self
                    .0
                    .iter()
                    .map(|(name, value)| name.len() + value.len())
                    .sum::<usize>()
                    + name.len()
                    + value.len()
                    <= 64 * 1024,
            "launch environment capacity exceeded"
        );
        self.0.push((name, value));
        Ok(())
    }
}
