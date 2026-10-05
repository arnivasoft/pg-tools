#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PgVersion(pub u32);

impl PgVersion {
    pub fn major(self) -> u32 {
        self.0 / 10_000
    }
}

/// Sürüme göre SQL varyantları: `(en düşük ana sürüm, sql)`, ana sürüme göre artan sırada.
pub struct VersionedSql(pub &'static [(u32, &'static str)]);

impl VersionedSql {
    pub fn pick(&self, version: PgVersion) -> &'static str {
        self.0
            .iter()
            .rev()
            .find(|(min, _)| version.major() >= *min)
            .or_else(|| self.0.first())
            .map(|(_, sql)| *sql)
            .expect("VersionedSql en az bir varyant içermeli")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static Q: VersionedSql = VersionedSql(&[(10, "pg10"), (12, "pg12"), (18, "pg18")]);

    #[test]
    fn major_from_version_num() {
        assert_eq!(PgVersion(100023).major(), 10);
        assert_eq!(PgVersion(160004).major(), 16);
        assert_eq!(PgVersion(180000).major(), 18);
    }

    #[test]
    fn picks_highest_matching_variant() {
        assert_eq!(Q.pick(PgVersion(100000)), "pg10");
        assert_eq!(Q.pick(PgVersion(110005)), "pg10");
        assert_eq!(Q.pick(PgVersion(120000)), "pg12");
        assert_eq!(Q.pick(PgVersion(170002)), "pg12");
        assert_eq!(Q.pick(PgVersion(180001)), "pg18");
        assert_eq!(Q.pick(PgVersion(190000)), "pg18");
        assert_eq!(Q.pick(PgVersion(90600)), "pg10", "eski sürüm en eski varyantı kullanır");
    }
}
