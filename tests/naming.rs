use ikigai_inquiry::{Action, Layers, Policy, PENDING_VIEW, SPAM_VIEW};

#[test]
fn owner_layers_are_singular_and_type_first() {
    let layers = Layers::for_owner("brian").unwrap();
    assert_eq!(layers.inquiry, "inquiry-brian");
    assert_eq!(layers.spam, "inquiry-spam-brian");
    assert_eq!(layers.holding, "inquiry-hold-brian");
    assert_eq!(layers.examples, "inquiry-example-brian");
    assert_eq!(layers.intents, "inquiry-intent-brian");
    let policy = Policy::for_owner("brian").unwrap();
    assert_eq!(policy.holding, layers.holding);
    assert_eq!(policy.examples, layers.examples);
}

#[test]
fn resource_names_have_named_partitions() {
    assert_eq!(Action::Remove.iri(), "urn:iki:inquiry:action:name:remove");
    assert_eq!(PENDING_VIEW, "urn:iki:inquiry:view:name:pending");
    assert_eq!(SPAM_VIEW, "urn:iki:inquiry:view:name:spam");
}

#[test]
fn owner_names_fit_the_hosts_limit() {
    let owner = "abcdefghijkl";
    let layers = Layers::for_owner(owner).unwrap();
    assert_eq!(layers.inquiry, "inq-abcdefghijkl");
    assert_eq!(layers.examples, "inq-example-abcdefghijkl");
    assert_eq!(Policy::for_owner(owner).unwrap().examples, layers.examples);
    assert!(Layers::for_owner("abcdefghijklm").is_err());
    assert!(Policy::for_owner("abcdefghijklm").is_err());
}

#[test]
fn owner_prefix_boundaries_and_invalid_owners_are_shared() {
    for (owner, prefix) in [("abcdefgh", "inquiry"), ("abcdefghi", "inq")] {
        let layers = Layers::for_owner(owner).unwrap();
        let policy = Policy::for_owner(owner).unwrap();
        assert_eq!(layers.inquiry, format!("{prefix}-{owner}"));
        assert_eq!(policy.inquiry, layers.inquiry);
        assert_eq!(policy.spam, layers.spam);
        assert_eq!(policy.holding, layers.holding);
        assert_eq!(policy.examples, layers.examples);
        for name in [
            &layers.inquiry,
            &layers.spam,
            &layers.holding,
            &layers.examples,
            &layers.intents,
        ] {
            assert!(name.len() <= 24);
            assert!(name.starts_with(&format!("{prefix}-")));
        }
    }
    for owner in ["", "-a", "A", "a:b", "a b", "é"] {
        assert!(Layers::for_owner(owner).is_err(), "{owner}");
        assert!(Policy::for_owner(owner).is_err(), "{owner}");
    }
}
