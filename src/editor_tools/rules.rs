//! The rule builder of the *Transform Styles* dialog (CE-133): which kinds of rule exist, which fields each one
//! asks for, and how typed values become a [`CssTransform`]. No GTK in here.

use crate::i18n::tr;
use bookx_editing::transform_css::CssTransform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    ChangeValue,
    SetProperty,
    RemoveProperty,
    RemoveWhenValue,
    ScaleFonts,
    FamilyItalic,
    RenameFamily,
    ReplaceColor,
}

/// One input of a rule form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub label: &'static str,
    pub hint: &'static str,
}

impl RuleKind {
    pub const ALL: [RuleKind; 8] = [
        RuleKind::ChangeValue,
        RuleKind::SetProperty,
        RuleKind::RemoveProperty,
        RuleKind::RemoveWhenValue,
        RuleKind::ScaleFonts,
        RuleKind::FamilyItalic,
        RuleKind::RenameFamily,
        RuleKind::ReplaceColor,
    ];

    pub fn label(self) -> &'static str {
        match self {
            RuleKind::ChangeValue => "If a property has a value, change it",
            RuleKind::SetProperty => "Set a property everywhere",
            RuleKind::RemoveProperty => "Remove a property",
            RuleKind::RemoveWhenValue => "Remove a property when it has a value",
            RuleKind::ScaleFonts => "Multiply all font sizes",
            RuleKind::FamilyItalic => "Make a font family italic",
            RuleKind::RenameFamily => "Rename a font family",
            RuleKind::ReplaceColor => "Change one colour into another",
        }
    }

    /// The short verb phrase for the combo row; [`RuleKind::label`] is the longer sentence shown beneath it.
    pub fn short_label(self) -> &'static str {
        match self {
            RuleKind::ChangeValue => "Change property value",
            RuleKind::SetProperty => "Set property",
            RuleKind::RemoveProperty => "Remove property",
            RuleKind::RemoveWhenValue => "Remove property by value",
            RuleKind::ScaleFonts => "Scale font sizes",
            RuleKind::FamilyItalic => "Italicise family",
            RuleKind::RenameFamily => "Rename family",
            RuleKind::ReplaceColor => "Replace colour",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            RuleKind::ChangeValue => "change",
            RuleKind::SetProperty => "set",
            RuleKind::RemoveProperty => "remove",
            RuleKind::RemoveWhenValue => "remove-value",
            RuleKind::ScaleFonts => "scale",
            RuleKind::FamilyItalic => "italic",
            RuleKind::RenameFamily => "rename",
            RuleKind::ReplaceColor => "color",
        }
    }

    pub fn from_id(id: &str) -> Option<RuleKind> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }

    pub fn fields(self) -> &'static [Field] {
        const PROP: Field = Field {
            label: "Property",
            hint: "color, margin-left, margin*",
        };
        match self {
            RuleKind::ChangeValue => &[
                PROP,
                Field {
                    label: "Is",
                    hint: "red",
                },
                Field {
                    label: "Change it to",
                    hint: "green",
                },
            ],
            RuleKind::SetProperty => &[
                PROP,
                Field {
                    label: "New value",
                    hint: "left",
                },
            ],
            RuleKind::RemoveProperty => &[PROP],
            RuleKind::RemoveWhenValue => &[
                PROP,
                Field {
                    label: "Value",
                    hint: "none",
                },
            ],
            RuleKind::ScaleFonts => &[Field {
                label: "Factor",
                hint: "1.2",
            }],
            RuleKind::FamilyItalic => &[Field {
                label: "Font family",
                hint: "Georgia",
            }],
            RuleKind::RenameFamily => &[
                Field {
                    label: "Family",
                    hint: "Arial",
                },
                Field {
                    label: "New name",
                    hint: "Helvetica",
                },
            ],
            RuleKind::ReplaceColor => &[
                Field {
                    label: "Colour",
                    hint: "#ff0000 or red",
                },
                Field {
                    label: "New colour",
                    hint: "#00aa00",
                },
            ],
        }
    }

    /// Builds the rule from the typed values (one per field, in order).
    pub fn build(self, vals: &[String]) -> Result<CssTransform, String> {
        let get = |i: usize| vals.get(i).map(|v| v.trim()).unwrap_or("");
        let need = |i: usize| -> Result<String, String> {
            let v = get(i);
            if v.is_empty() {
                Err(format!("Fill in “{}”", self.fields()[i].label))
            } else {
                Ok(v.to_string())
            }
        };
        let property = |i: usize| -> Result<String, String> {
            let v = need(i)?.to_ascii_lowercase();
            let body = v.strip_suffix('*').unwrap_or(&v);
            if body.is_empty() && v != "*"
                || body
                    .chars()
                    .any(|c| !(c.is_ascii_alphanumeric() || c == '-'))
            {
                return Err(format!("“{v}” is not a property name"));
            }
            Ok(v)
        };
        Ok(match self {
            RuleKind::ChangeValue => CssTransform::ChangeValue {
                name: property(0)?,
                from: need(1)?,
                to: need(2)?,
            },
            RuleKind::SetProperty => CssTransform::SetProperty {
                name: property(0)?,
                value: need(1)?,
            },
            RuleKind::RemoveProperty => CssTransform::RemoveProperty { name: property(0)? },
            RuleKind::RemoveWhenValue => CssTransform::RemovePropertyWithValue {
                name: property(0)?,
                value: need(1)?,
            },
            RuleKind::ScaleFonts => {
                let f: f64 = need(0)?
                    .replace(',', ".")
                    .parse()
                    .map_err(|_| tr("The factor must be a number such as 1.2"))?;
                if !(0.05..=20.0).contains(&f) {
                    return Err(tr("The factor must be between 0.05 and 20"));
                }
                CssTransform::ScaleFontSizes { factor: f }
            }
            RuleKind::FamilyItalic => CssTransform::make_family_italic(&need(0)?),
            RuleKind::RenameFamily => CssTransform::RenameFontFamily {
                from: need(0)?,
                to: need(1)?,
            },
            RuleKind::ReplaceColor => {
                let from = need(0)?;
                if bookx_editing::transform_css::parse_color(&from).is_none() {
                    return Err(format!("“{from}” is not a colour"));
                }
                CssTransform::ReplaceColor { from, to: need(1)? }
            }
        })
    }
}

/// Parses a dev/test description `kind|v1|v2|v3`.
pub fn parse_rule(text: &str) -> Result<CssTransform, String> {
    let mut it = text.split('|');
    let kind = it
        .next()
        .and_then(RuleKind::from_id)
        .ok_or_else(|| format!("Unknown rule kind in “{text}”"))?;
    let vals: Vec<String> = it.map(str::to_string).collect();
    kind.build(&vals)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn every_kind_builds_from_its_fields() {
        assert_eq!(
            RuleKind::ChangeValue
                .build(&v(&["Color", " red ", "green"]))
                .unwrap(),
            CssTransform::ChangeValue {
                name: "color".into(),
                from: "red".into(),
                to: "green".into()
            }
        );
        assert_eq!(
            RuleKind::ScaleFonts.build(&v(&["1,5"])).unwrap(),
            CssTransform::ScaleFontSizes { factor: 1.5 }
        );
        assert_eq!(
            RuleKind::FamilyItalic.build(&v(&["Georgia"])).unwrap(),
            CssTransform::make_family_italic("Georgia")
        );
        assert!(matches!(
            RuleKind::RemoveProperty.build(&v(&["margin*"])),
            Ok(CssTransform::RemoveProperty { .. })
        ));
        for k in RuleKind::ALL {
            assert_eq!(RuleKind::from_id(k.id()), Some(k));
            assert!(!k.label().is_empty() && !k.fields().is_empty());
            // an empty form never builds
            assert!(k.build(&vec![String::new(); k.fields().len()]).is_err());
        }
    }

    #[test]
    fn bad_input_is_explained() {
        assert_eq!(
            RuleKind::ChangeValue
                .build(&v(&["color", "", "green"]))
                .unwrap_err(),
            "Fill in “Is”"
        );
        assert!(
            RuleKind::RemoveProperty
                .build(&v(&["a b"]))
                .unwrap_err()
                .contains("not a property")
        );
        assert!(RuleKind::ScaleFonts.build(&v(&["big"])).is_err());
        assert!(RuleKind::ScaleFonts.build(&v(&["0"])).is_err());
        assert!(
            RuleKind::ReplaceColor
                .build(&v(&["notacolour", "red"]))
                .unwrap_err()
                .contains("not a colour")
        );
        assert!(RuleKind::ReplaceColor.build(&v(&["#f00", "#0a0"])).is_ok());
    }

    #[test]
    fn dev_descriptions_parse() {
        assert!(matches!(
            parse_rule("scale|2"),
            Ok(CssTransform::ScaleFontSizes { .. })
        ));
        assert!(matches!(
            parse_rule("change|color|red|green"),
            Ok(CssTransform::ChangeValue { .. })
        ));
        assert!(parse_rule("nope|1").is_err());
    }

    #[test]
    fn the_short_labels_fit_a_combo_row_and_differ() {
        let shorts: Vec<&str> = RuleKind::ALL.iter().map(|k| k.short_label()).collect();
        assert!(shorts.iter().all(|s| s.chars().count() <= 26), "{shorts:?}");
        let mut sorted = shorts.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), shorts.len());
        assert!(
            RuleKind::ALL
                .iter()
                .all(|k| k.label().len() > k.short_label().len())
        );
    }
}
