//! The published `inq:` vocabulary (`src/vocabulary.ttl`, served at
//! https://ikigai-rs.dev/ns/inquiry) matches the layer files the crate ships: what the shapes
//! admit and the context writes is what the vocabulary defines, with the same ranges.

use std::collections::{BTreeMap, BTreeSet};

use ikigai_inquiry::vocab::{
    CONTEXT, EXAMPLE_SHAPES, INQ, INQUIRY_SHAPES, INTENT_SHAPES, VOCABULARY,
};
use oxrdf::{NamedOrBlankNode, Term, Triple};
use oxttl::TurtleParser;

const ONTOLOGY: &str = "https://ikigai-rs.dev/ns/inquiry";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_PROPERTY: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#Property";
const RDFS_CLASS: &str = "http://www.w3.org/2000/01/rdf-schema#Class";
const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
const RDFS_COMMENT: &str = "http://www.w3.org/2000/01/rdf-schema#comment";
const RDFS_RANGE: &str = "http://www.w3.org/2000/01/rdf-schema#range";
const RDFS_DOMAIN: &str = "http://www.w3.org/2000/01/rdf-schema#domain";
const OWL_ONTOLOGY: &str = "http://www.w3.org/2002/07/owl#Ontology";
const OWL_VERSION_INFO: &str = "http://www.w3.org/2002/07/owl#versionInfo";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
const SH: &str = "http://www.w3.org/ns/shacl#";

fn parse(turtle: &str) -> Vec<Triple> {
    TurtleParser::new()
        .for_slice(turtle.as_bytes())
        .map(|triple| triple.expect("the Turtle parses"))
        .collect()
}

fn subject(triple: &Triple) -> String {
    match &triple.subject {
        NamedOrBlankNode::NamedNode(node) => node.as_str().to_string(),
        NamedOrBlankNode::BlankNode(node) => format!("_:{}", node.as_str()),
    }
}

fn iri(term: &Term) -> Option<&str> {
    match term {
        Term::NamedNode(node) => Some(node.as_str()),
        _ => None,
    }
}

fn objects<'a>(graph: &'a [Triple], s: &str, p: &str) -> Vec<&'a Term> {
    graph
        .iter()
        .filter(|t| subject(t) == s && t.predicate.as_str() == p)
        .map(|t| &t.object)
        .collect()
}

fn one_iri(graph: &[Triple], s: &str, p: &str) -> Option<String> {
    let found = objects(graph, s, p);
    assert!(found.len() <= 1, "{s}: at most one {p}: {found:?}");
    found.first().map(|o| iri(o).expect("an IRI").to_string())
}

/// Every `inq:` IRI a graph names, in any position.
fn inq_terms(graph: &[Triple]) -> BTreeSet<String> {
    let mut terms = BTreeSet::new();
    for t in graph {
        for name in [
            Some(subject(t).as_str()),
            Some(t.predicate.as_str()),
            iri(&t.object),
        ]
        .into_iter()
        .flatten()
        {
            if name.starts_with(INQ) {
                terms.insert(name.to_string());
            }
        }
    }
    terms
}

/// Every `inq:` property each shape file admits: its datatype, and the shapes' target classes.
fn shape_paths() -> BTreeMap<String, Vec<(String, String)>> {
    let mut paths: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for shapes in [INQUIRY_SHAPES, EXAMPLE_SHAPES, INTENT_SHAPES] {
        let graph = parse(shapes);
        for shape in graph
            .iter()
            .filter(|t| t.predicate.as_str() == format!("{SH}targetClass"))
        {
            let shape_id = subject(shape);
            let class = iri(&shape.object).unwrap().to_string();
            for property in objects(&graph, &shape_id, &format!("{SH}property")) {
                let Term::BlankNode(node) = property else {
                    panic!("a property shape")
                };
                let node = format!("_:{}", node.as_str());
                let path = one_iri(&graph, &node, &format!("{SH}path")).unwrap();
                if !path.starts_with(INQ) {
                    continue;
                }
                let datatype = one_iri(&graph, &node, &format!("{SH}datatype"))
                    .unwrap_or_else(|| panic!("{path}: a datatype in {shape_id}"));
                paths
                    .entry(path)
                    .or_default()
                    .push((class.clone(), datatype));
            }
        }
    }
    paths
}

/// The context's `inq:` terms, with the type each is coerced to (`xsd:string` when none).
fn context_terms() -> BTreeMap<String, String> {
    let context: serde_json::Value = serde_json::from_str(CONTEXT).unwrap();
    let mut terms = BTreeMap::new();
    for (_, definition) in context["@context"].as_object().unwrap() {
        let (id, kind) = match definition {
            serde_json::Value::String(id) => (id.as_str(), None),
            serde_json::Value::Object(o) => (o["@id"].as_str().unwrap(), o["@type"].as_str()),
            _ => continue,
        };
        let Some(local) = id.strip_prefix("inq:") else {
            continue;
        };
        let kind = kind.map_or(XSD_STRING.to_string(), |k| k.replace("xsd:", XSD));
        terms.insert(format!("{INQ}{local}"), kind);
    }
    terms
}

fn defined(vocabulary: &[Triple]) -> BTreeSet<String> {
    vocabulary
        .iter()
        .filter(|t| t.predicate.as_str() == RDF_TYPE && subject(t).starts_with(INQ))
        .map(subject)
        .collect()
}

/// Every `inq:` term the shapes or the context name is defined, and every defined term is named
/// by one of them: the published document neither lags the layers nor promises what they never
/// write.
#[test]
fn every_term_the_layers_use_is_defined_and_no_other() {
    let vocabulary = parse(VOCABULARY);
    let defined = defined(&vocabulary);
    let mut used = BTreeSet::new();
    for shapes in [INQUIRY_SHAPES, EXAMPLE_SHAPES, INTENT_SHAPES] {
        used.extend(inq_terms(&parse(shapes)));
    }
    used.extend(context_terms().into_keys());
    assert_eq!(
        defined, used,
        "the vocabulary defines exactly the inq: terms the layers use"
    );
    assert_eq!(
        inq_terms(&vocabulary),
        defined,
        "every inq: term the vocabulary names, it defines"
    );
}

/// A class is a class and a property a property, each with one label and one comment; a
/// property has one range and at most one domain.
#[test]
fn every_term_is_described() {
    let vocabulary = parse(VOCABULARY);
    let terms = defined(&vocabulary);
    assert!(terms.len() > 20, "the whole vocabulary parsed: {terms:?}");
    for term in std::iter::once(ONTOLOGY.to_string()).chain(terms.iter().cloned()) {
        assert_eq!(
            objects(&vocabulary, &term, RDFS_LABEL).len(),
            1,
            "{term}: one label"
        );
        assert_eq!(
            objects(&vocabulary, &term, RDFS_COMMENT).len(),
            1,
            "{term}: one comment"
        );
    }
    for term in &terms {
        let kinds: Vec<_> = objects(&vocabulary, term, RDF_TYPE)
            .into_iter()
            .filter_map(iri)
            .collect();
        assert!(
            kinds == [RDFS_CLASS] || kinds == [RDF_PROPERTY],
            "{term}: {kinds:?}"
        );
        let local = &term[INQ.len()..];
        let is_class = local.starts_with(|c: char| c.is_ascii_uppercase());
        assert_eq!(
            kinds == [RDFS_CLASS],
            is_class,
            "{term}: classes are capitalized"
        );
        if !is_class {
            assert!(
                one_iri(&vocabulary, term, RDFS_RANGE).is_some(),
                "{term}: one range"
            );
            one_iri(&vocabulary, term, RDFS_DOMAIN);
        }
    }
}

/// Each property's range is the datatype every shape demands and the type the context
/// coerces to; a declared domain is the target class of every shape that admits the property,
/// since a domain is an inference about every subject.
#[test]
fn ranges_and_domains_are_what_the_layers_write() {
    let vocabulary = parse(VOCABULARY);
    let context = context_terms();
    for (property, uses) in shape_paths() {
        let range = one_iri(&vocabulary, &property, RDFS_RANGE).unwrap();
        for (class, datatype) in &uses {
            assert_eq!(
                &range, datatype,
                "{property}: range as the shape for {class} demands"
            );
        }
        if let Some(kind) = context.get(&property) {
            assert_eq!(&range, kind, "{property}: range as the context coerces");
        }
        let classes: BTreeSet<_> = uses.iter().map(|(class, _)| class.clone()).collect();
        match one_iri(&vocabulary, &property, RDFS_DOMAIN) {
            Some(domain) => assert_eq!(
                classes,
                BTreeSet::from([domain]),
                "{property}: a domain only where every shape that admits it targets that class"
            ),
            None => assert!(
                classes.len() > 1,
                "{property}: admitted only on {classes:?}, so it should declare that domain"
            ),
        }
    }
}

/// The deployed /ns/inquiry is a copy of `VOCABULARY`, and `owl:versionInfo` is how one deploy
/// is told apart from the last: devtools' `ns-drift` reads it on the live and the repo side.
/// It is the crate version, so a release bump must move it. Read as RDF and as the one text
/// line the drift scan finds.
#[test]
fn ontology_version_is_the_crate_version() {
    let vocabulary = parse(VOCABULARY);
    let kinds: Vec<_> = objects(&vocabulary, ONTOLOGY, RDF_TYPE)
        .into_iter()
        .filter_map(iri)
        .collect();
    assert_eq!(
        kinds,
        [OWL_ONTOLOGY],
        "{ONTOLOGY} is declared an owl:Ontology"
    );
    let versions = objects(&vocabulary, ONTOLOGY, OWL_VERSION_INFO);
    let [Term::Literal(version)] = versions.as_slice() else {
        panic!("exactly one owl:versionInfo literal: {versions:?}");
    };
    assert_eq!(version.datatype().as_str(), XSD_STRING, "a plain string");
    assert_eq!(
        version.value(),
        env!("CARGO_PKG_VERSION"),
        "vocabulary.ttl's owl:versionInfo must be bumped with the crate version"
    );
    let lines: Vec<&str> = VOCABULARY
        .lines()
        .filter(|l| l.trim_start().starts_with("owl:versionInfo"))
        .collect();
    assert_eq!(
        lines,
        [format!(
            "    owl:versionInfo \"{}\" .",
            env!("CARGO_PKG_VERSION")
        )],
        "exactly one owl:versionInfo line, in the form the drift scan reads"
    );
}
