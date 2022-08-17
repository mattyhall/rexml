pub mod ts_float_seconds {
    use serde::de;
    use chrono::{DateTime, Utc, NaiveDateTime};

    pub struct SecondsTimestampVisitor;

    pub fn deserialize<'de, D>(d: D) -> Result<DateTime<Utc>, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        d.deserialize_f64(SecondsTimestampVisitor)
    }

    impl<'de> de::Visitor<'de> for SecondsTimestampVisitor {
        type Value = DateTime<Utc>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a unix timestamp in seconds")
        }

        /// Deserialize a timestamp in seconds since the epoch
        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let naive = NaiveDateTime::from_timestamp(value as i64, 0);
            Ok(DateTime::from_utc(naive, Utc))
        }
    }
}

