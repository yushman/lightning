use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

const MAX_MESSAGE_LEN: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Fail,
    Error,
    Skip,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TestResult {
    pub class_name: String,
    pub name: String,
    pub status: Status,
    pub time_ms: u64,
    pub message: Option<String>,
}

fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (a.key.as_ref() == name.as_bytes())
            .then(|| {
                a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .ok()
                    .map(|v| v.into_owned())
            })
            .flatten()
    })
}

fn time_ms(e: &BytesStart) -> u64 {
    attr(e, "time")
        .and_then(|t| t.parse::<f64>().ok())
        .map(|t| (t * 1000.0).round().max(0.0) as u64)
        .unwrap_or(0)
}

/// Parses one JUnit XML document into flat test results.
pub fn parse(xml: &str) -> Result<Vec<TestResult>, String> {
    let mut reader = Reader::from_str(xml);
    let mut results = Vec::new();
    let mut suites: Vec<String> = Vec::new();
    // Current <testcase>, if any: (class_name, name, time_ms, status, message).
    let mut case: Option<(String, String, u64, Status, Option<String>)> = None;

    loop {
        let event = reader.read_event().map_err(|e| e.to_string())?;
        match &event {
            Event::Start(e) | Event::Empty(e) => {
                let empty = matches!(event, Event::Empty(_));
                match e.name().as_ref() {
                    b"testsuite" if !empty => {
                        suites.push(attr(e, "name").unwrap_or_default());
                    }
                    b"testcase" => {
                        let class_name = attr(e, "classname")
                            .or_else(|| suites.last().cloned())
                            .unwrap_or_default();
                        let name = attr(e, "name").unwrap_or_default();
                        let ms = time_ms(e);
                        if empty {
                            results.push(TestResult {
                                class_name,
                                name,
                                status: Status::Pass,
                                time_ms: ms,
                                message: None,
                            });
                        } else {
                            case = Some((class_name, name, ms, Status::Pass, None));
                        }
                    }
                    b"failure" | b"error" | b"skipped" if case.is_some() => {
                        let status = match e.name().as_ref() {
                            b"failure" => Status::Fail,
                            b"error" => Status::Error,
                            _ => Status::Skip,
                        };
                        let message =
                            attr(e, "message").map(|m| m.chars().take(MAX_MESSAGE_LEN).collect());
                        let c = case.as_mut().unwrap();
                        c.3 = status;
                        c.4 = message;
                    }
                    _ => {}
                }
            }
            Event::End(e) => match e.name().as_ref() {
                b"testsuite" => {
                    suites.pop();
                }
                b"testcase" => {
                    if let Some((class_name, name, ms, status, message)) = case.take() {
                        results.push(TestResult {
                            class_name,
                            name,
                            status,
                            time_ms: ms,
                            message,
                        });
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_statuses_and_metadata() {
        let xml = r#"<?xml version="1.0"?>
<testsuite name="com.example.FooTest" tests="4" time="1.5">
  <testcase classname="com.example.FooTest" name="passes" time="0.123"/>
  <testcase classname="com.example.FooTest" name="fails" time="0.5">
    <failure message="expected 1 but was 2" type="AssertionError">stack</failure>
  </testcase>
  <testcase classname="com.example.FooTest" name="errors" time="0">
    <error message="boom"/>
  </testcase>
  <testcase classname="com.example.FooTest" name="skipped">
    <skipped/>
  </testcase>
</testsuite>"#;
        let r = parse(xml).unwrap();
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].status, Status::Pass);
        assert_eq!(r[0].time_ms, 123);
        assert_eq!(r[1].status, Status::Fail);
        assert_eq!(r[1].message.as_deref(), Some("expected 1 but was 2"));
        assert_eq!(r[2].status, Status::Error);
        assert_eq!(r[3].status, Status::Skip);
        assert_eq!(r[3].time_ms, 0);
    }

    #[test]
    fn falls_back_to_suite_name_for_class() {
        let xml = r#"<testsuites><testsuite name="OuterSuite">
  <testcase name="works" time="0.01"/>
</testsuite></testsuites>"#;
        let r = parse(xml).unwrap();
        assert_eq!(r[0].class_name, "OuterSuite");
        assert_eq!(r[0].name, "works");
    }

    #[test]
    fn rejects_malformed_xml() {
        assert!(parse("<testsuite><testcase").is_err());
    }
}
