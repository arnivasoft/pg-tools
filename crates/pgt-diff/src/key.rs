//! Nesne anahtarı ve iki taraf arasında eşleştirme.

use std::collections::BTreeMap;

use pgt_ddl::ObjectKind;

/// Anahtar sınıfı; sıralama da bu sırayla. Tablo ile görünüm aynı sınıftadır (aynı ad uzayı).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyClass {
    Relation,
    Type,
    Routine,
    Index,
    Trigger,
}

impl KeyClass {
    fn letter(self) -> &'static str {
        match self {
            KeyClass::Relation => "r",
            KeyClass::Type => "t",
            KeyClass::Routine => "f",
            KeyClass::Index => "i",
            KeyClass::Trigger => "g",
        }
    }
}

/// Nesne türünün anahtar sınıfı; karşılaştırmaya girmeyen türler için `None`.
pub fn class_of(kind: ObjectKind) -> Option<KeyClass> {
    match kind {
        ObjectKind::Table
        | ObjectKind::View
        | ObjectKind::MaterializedView
        | ObjectKind::Sequence => Some(KeyClass::Relation),
        ObjectKind::Type | ObjectKind::Domain => Some(KeyClass::Type),
        ObjectKind::Function | ObjectKind::Procedure => Some(KeyClass::Routine),
        ObjectKind::Index => Some(KeyClass::Index),
        ObjectKind::Trigger => Some(KeyClass::Trigger),
        _ => None,
    }
}

/// Katalogdan listelenen bir nesne.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub kind: ObjectKind,
    pub oid: u32,
    pub name: String,
    pub table: Option<String>,
    pub signature: Option<String>,
    pub is_partition: bool,
    pub deps: Vec<u32>,
}

/// Opak nesne anahtarı: `(sınıf harfi, tablo ?? "", ad, imza ?? "")` JSON dizisi.
/// Sınıfı bilinmeyen tür için boş sınıf harfi kullanılır (eşleştirmeye zaten girmez).
pub fn item_key(l: &Listed) -> String {
    let class = class_of(l.kind).map(KeyClass::letter).unwrap_or("");
    serde_json::to_string(&(
        class,
        l.table.as_deref().unwrap_or(""),
        l.name.as_str(),
        l.signature.as_deref().unwrap_or(""),
    ))
    .expect("demet serileştirilebilir")
}

/// Aynı anahtarlı iki taraf çifti (taraflardan biri yoksa `None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair<'a> {
    pub key: String,
    pub source: Option<&'a Listed>,
    pub target: Option<&'a Listed>,
}

type SortKey<'a> = (KeyClass, &'a str, &'a str, &'a str);

fn sort_key(l: &Listed) -> Option<SortKey<'_>> {
    Some((
        class_of(l.kind)?,
        l.table.as_deref().unwrap_or(""),
        l.name.as_str(),
        l.signature.as_deref().unwrap_or(""),
    ))
}

/// İki tarafı anahtarla eşleştirir. Sıra: sınıf, tablo, ad, imza (bayt sırası); deterministiktir.
/// Sınıfı olmayan türler atlanır. Aynı taraftaki yinelenen anahtarda ilk kayıt kalır.
pub fn match_objects<'a>(source: &'a [Listed], target: &'a [Listed]) -> Vec<Pair<'a>> {
    type Slot<'a> = (Option<&'a Listed>, Option<&'a Listed>);
    let mut map: BTreeMap<SortKey<'a>, Slot<'a>> = BTreeMap::new();
    for l in source {
        if let Some(k) = sort_key(l) {
            map.entry(k).or_default().0.get_or_insert(l);
        }
    }
    for l in target {
        if let Some(k) = sort_key(l) {
            map.entry(k).or_default().1.get_or_insert(l);
        }
    }
    map.into_values()
        .map(|(s, t)| {
            let key = item_key(s.or(t).expect("en az bir taraf dolu"));
            Pair {
                key,
                source: s,
                target: t,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(kind: ObjectKind, name: &str, table: Option<&str>, sig: Option<&str>) -> Listed {
        Listed {
            kind,
            oid: 1,
            name: name.into(),
            table: table.map(Into::into),
            signature: sig.map(Into::into),
            is_partition: false,
            deps: vec![],
        }
    }

    #[test]
    fn table_and_view_pair() {
        let s = [l(ObjectKind::Table, "x", None, None)];
        let t = [l(ObjectKind::View, "x", None, None)];
        let p = match_objects(&s, &t);
        assert_eq!(p.len(), 1);
        assert!(p[0].source.is_some() && p[0].target.is_some());
    }

    #[test]
    fn overloads_are_separate() {
        let s = [
            l(ObjectKind::Function, "f", None, Some("integer")),
            l(ObjectKind::Function, "f", None, Some("text")),
        ];
        let t = [l(ObjectKind::Function, "f", None, Some("integer"))];
        let p = match_objects(&s, &t);
        assert_eq!(p.len(), 2);
        assert!(p[0].target.is_some());
        assert!(p[1].target.is_none());
    }

    #[test]
    fn same_index_name_on_different_tables() {
        let s = [
            l(ObjectKind::Index, "i", Some("a"), None),
            l(ObjectKind::Index, "i", Some("b"), None),
        ];
        let p = match_objects(&s, &[]);
        assert_eq!(p.len(), 2);
        assert_ne!(p[0].key, p[1].key);
    }

    #[test]
    fn table_and_type_are_distinct() {
        let s = [l(ObjectKind::Table, "x", None, None)];
        let t = [l(ObjectKind::Type, "x", None, None)];
        let p = match_objects(&s, &t);
        assert_eq!(p.len(), 2);
        assert!(p[0].source.is_some() && p[0].target.is_none());
        assert!(p[1].source.is_none() && p[1].target.is_some());
    }

    #[test]
    fn keys_are_collision_safe() {
        let a = l(ObjectKind::Table, "a\u{1f}b", None, None);
        let b = l(ObjectKind::Index, "a", Some("b"), None);
        let c = l(ObjectKind::Table, "a\",\"b", None, None);
        let d = l(ObjectKind::Table, "a", Some("b"), None);
        let keys = [item_key(&a), item_key(&b), item_key(&c), item_key(&d)];
        for i in 0..keys.len() {
            for j in i + 1..keys.len() {
                assert_ne!(keys[i], keys[j]);
            }
        }
    }

    #[test]
    fn deterministic_class_then_name_order() {
        let s = [
            l(ObjectKind::Trigger, "tg", Some("t"), None),
            l(ObjectKind::Index, "i", Some("t"), None),
            l(ObjectKind::Function, "f", None, Some("")),
            l(ObjectKind::Domain, "d", None, None),
            l(ObjectKind::Sequence, "b", None, None),
            l(ObjectKind::Table, "a", None, None),
            l(ObjectKind::Schema, "ignored", None, None),
        ];
        let p = match_objects(&s, &[]);
        let names: Vec<_> = p.iter().map(|x| x.source.unwrap().name.as_str()).collect();
        assert_eq!(names, ["a", "b", "d", "f", "i", "tg"]);
    }
}
