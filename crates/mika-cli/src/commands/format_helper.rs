use crate::cli::OutputFormat;

/// Print a serializable value as JSON or YAML. Returns `true` if it handled the format,
/// `false` if the caller should render text output.
pub fn print_structured<T: serde::Serialize>(
    format: &OutputFormat,
    value: &T,
) -> anyhow::Result<bool> {
    match format {
        OutputFormat::Text => Ok(false),
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(value)?);
            Ok(true)
        }
        OutputFormat::Yaml => {
            print!("{}", serde_yaml::to_string(value)?);
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::ValueEnum;
    use serde::Serialize;

    #[derive(Serialize, serde::Deserialize)]
    struct Sample {
        name: String,
        count: u32,
    }

    #[test]
    fn text_format_returns_false() {
        let result = print_structured(&OutputFormat::Text, &"anything").unwrap();
        assert!(!result);
    }

    #[test]
    fn json_format_returns_true() {
        let sample = Sample {
            name: "test".into(),
            count: 42,
        };
        let result = print_structured(&OutputFormat::Json, &sample).unwrap();
        assert!(result);
    }

    #[test]
    fn yaml_format_returns_true() {
        let sample = Sample {
            name: "test".into(),
            count: 42,
        };
        let result = print_structured(&OutputFormat::Yaml, &sample).unwrap();
        assert!(result);
    }

    #[test]
    fn output_format_yaml_variant_parseable() {
        let yaml = OutputFormat::from_str("yaml", true).unwrap();
        assert!(matches!(yaml, OutputFormat::Yaml));
    }

    #[test]
    fn output_format_all_variants_present() {
        let variants: Vec<_> = OutputFormat::value_variants()
            .iter()
            .map(|v| v.to_possible_value().unwrap().get_name().to_string())
            .collect();
        assert!(variants.contains(&"text".to_string()));
        assert!(variants.contains(&"json".to_string()));
        assert!(variants.contains(&"yaml".to_string()));
        assert_eq!(variants.len(), 3);
    }

    #[test]
    fn yaml_serialization_produces_valid_yaml() {
        let sample = Sample {
            name: "hello".into(),
            count: 7,
        };
        let yaml_str = serde_yaml::to_string(&sample).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml_str).unwrap();
        assert_eq!(parsed["name"], serde_yaml::Value::String("hello".into()));
    }

    #[test]
    fn yaml_serialization_roundtrips_vec() {
        let items = vec![
            Sample {
                name: "a".into(),
                count: 1,
            },
            Sample {
                name: "b".into(),
                count: 2,
            },
        ];
        let yaml_str = serde_yaml::to_string(&items).unwrap();
        let parsed: Vec<Sample> = serde_yaml::from_str(&yaml_str).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "a");
        assert_eq!(parsed[1].count, 2);
    }
}
