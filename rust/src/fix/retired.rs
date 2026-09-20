//! What the specification retired, and what stands in for it.
//!
//! FIX 4.3, 4.4, 5.0 and 5.0 SP1 each retired fields and values and said what
//! replaced them - their Appendix 6-F "Replaced features" and Appendix 6-E
//! "Deprecated features": `Rule80A(47)` became `OrderCapacity(528)` beside
//! `OrderRestrictions(529)`, the partial-fill values of `ExecType(150)` folded
//! into `Trade`, `ExecBroker(76)` became one `Parties` occurrence with role
//! `1`. Those retirements are facts about FIX itself, the same for every
//! dictionary that declares the tags, so they are held here as one table
//! keyed by the retired tag, and the [restatement](super::latest) applies
//! them as it builds a message - exactly as it applies the rules a registry
//! states of its own on a field's [`FIX:replacements`](super::replacements),
//! through the same writes and the same checks, with nothing parsed, bound
//! or evaluated per message.
//!
//! # One owner
//!
//! A field's own `FIX:replacements` document wins whole over this table: a
//! registry that states what `Rule80A(47)` becomes restates by what it
//! states, and this table does not fill in behind it. A registry that states
//! nothing on a retired field restates by the specification. The shipped
//! dictionary states nothing, so what it carried as documents before lives
//! here and nowhere else.
//!
//! # Order
//!
//! Entries under one tag are in the specification's order and the order is
//! semantic: the first entry whose condition the held value meets answers,
//! so a catch-all stating no condition comes last, and `ExecInst(18)`'s
//! FIX 4.4 peg rule stands before its FIX 5.0 peg price types because the
//! `R` the first writes is what the second reads. The tags are sorted, which
//! is what lets a lookup be a binary search.

/// One retirement of one value, or of a whole field, of the tag it is
/// listed under.
pub(super) struct Rule {
    /// The held value the rule is about, where it is about one.
    pub(super) when: When,
    /// The message types the rule applies within; empty for every type.
    pub(super) msgtypes: &'static [&'static str],
    /// The repeating group the rule applies inside, where it applies inside
    /// one only and never at the root.
    pub(super) within: Option<&'static str>,
    /// What the rule fills, all or nothing.
    pub(super) fills: &'static [Fill],
}

/// The condition on the held value.
pub(super) enum When {
    /// Every value: the field itself was retired.
    Any,
    /// The value spells this code.
    Equals(&'static str),
    /// One of the value's codes is this one: a `MultipleCharValue` source,
    /// several codes in one text, matched by token.
    Contains(&'static str),
}

/// One target a rule fills.
pub(super) enum Fill {
    /// A constant, read as the target's field reads its wire text; written
    /// over the rule's own source it replaces the token the condition named.
    Constant { tag: i32, text: &'static str },
    /// The source's own value, re-typed for the target.
    Source { tag: i32 },
    /// Another field's stated value at the same level; unstated, the rule
    /// does not apply.
    From { tag: i32, source: i32 },
    /// The wire texts of several fields at the same level joined, every
    /// part stated.
    Join { tag: i32, parts: &'static [Part] },
    /// One occurrence of a repeating group at this level, a member per
    /// fill; a member may itself be an occurrence.
    Occurrence {
        group: &'static str,
        members: &'static [Fill],
    },
}

/// One part of a join.
pub(super) enum Part {
    /// The field's wire text as it is.
    Text(i32),
    /// A number spelled with two digits, which is how a day completes a
    /// month-year.
    TwoDigits(i32),
}

/// The retirements of `tag`, in the specification's order; `None` where the
/// specification retired nothing of it.
pub(super) fn rules_of(tag: i32) -> Option<&'static [Rule]> {
    RULES
        .binary_search_by_key(&tag, |(held, _)| *held)
        .ok()
        .map(|at| RULES[at].1)
}

/// Every retired tag with its rules, sorted by tag.
///
/// The entries a tag lists in one appendix stand before the ones a later
/// appendix added, which is the order the specification retired them in.
pub(super) static RULES: &[(i32, &[Rule])] = &[
    (
        18,
        &[
            // ExecInst T is a PrimaryPeg with PegMoveType Fixed and PegScope Local (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Contains("T"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 835,
                        text: "1",
                    },
                    Fill::Constant {
                        tag: 840,
                        text: "1",
                    },
                    Fill::Constant { tag: 18, text: "R" },
                ],
            },
            // ExecInst L is PegPriceType 1 (FIX 5.0 Appendix 6-E)
            Rule {
                when: When::Contains("L"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 1094,
                    text: "1",
                }],
            },
            // ExecInst M is PegPriceType 2 (FIX 5.0 Appendix 6-E)
            Rule {
                when: When::Contains("M"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 1094,
                    text: "2",
                }],
            },
            // ExecInst O is PegPriceType 3 (FIX 5.0 Appendix 6-E)
            Rule {
                when: When::Contains("O"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 1094,
                    text: "3",
                }],
            },
            // ExecInst P is PegPriceType 4 (FIX 5.0 Appendix 6-E)
            Rule {
                when: When::Contains("P"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 1094,
                    text: "4",
                }],
            },
            // ExecInst R is PegPriceType 5 (FIX 5.0 Appendix 6-E)
            Rule {
                when: When::Contains("R"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 1094,
                    text: "5",
                }],
            },
            // ExecInst W is PegPriceType 7 (FIX 5.0 Appendix 6-E)
            Rule {
                when: When::Contains("W"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 1094,
                    text: "7",
                }],
            },
            // ExecInst a is PegPriceType 8 (FIX 5.0 Appendix 6-E)
            Rule {
                when: When::Contains("a"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 1094,
                    text: "8",
                }],
            },
            // ExecInst d is PegPriceType 9 (FIX 5.0 Appendix 6-E)
            Rule {
                when: When::Contains("d"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 1094,
                    text: "9",
                }],
            },
        ],
    ),
    (
        20,
        &[
            // ExecTransType Cancel is ExecType TradeCancel (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("1"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 150,
                    text: "H",
                }],
            },
            // ExecTransType Correct is ExecType TradeCorrect (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("2"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 150,
                    text: "G",
                }],
            },
            // ExecTransType Status is ExecType OrderStatus (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("3"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 150,
                    text: "I",
                }],
            },
        ],
    ),
    (
        37,
        &[Rule {
            when: When::Any,
            msgtypes: &["r"],
            within: None,
            fills: &[Fill::Source { tag: 1369 }],
        }],
    ),
    (
        40,
        &[
            // OrdType MarketOnClose is Market at TimeInForce AtTheClose (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("5"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant { tag: 40, text: "1" },
                    Fill::Constant { tag: 59, text: "7" },
                ],
            },
            // OrdType OnClose is Market at TimeInForce AtTheClose (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("A"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant { tag: 40, text: "1" },
                    Fill::Constant { tag: 59, text: "7" },
                ],
            },
            // OrdType LimitOnClose is Limit at TimeInForce AtTheClose (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("B"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant { tag: 40, text: "2" },
                    Fill::Constant { tag: 59, text: "7" },
                ],
            },
            // OrdType ForexMarket is Market on Product CURRENCY (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("C"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant { tag: 40, text: "1" },
                    Fill::Constant {
                        tag: 460,
                        text: "4",
                    },
                ],
            },
            // OrdType ForexLimit is Limit on Product CURRENCY (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("F"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant { tag: 40, text: "2" },
                    Fill::Constant {
                        tag: 460,
                        text: "4",
                    },
                ],
            },
            // OrdType ForexPreviouslyQuoted is PreviouslyQuoted on Product CURRENCY (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("H"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant { tag: 40, text: "D" },
                    Fill::Constant {
                        tag: 460,
                        text: "4",
                    },
                ],
            },
        ],
    ),
    (
        46,
        &[Rule {
            when: When::Any,
            msgtypes: &[],
            within: None,
            fills: &[Fill::Source { tag: 55 }],
        }],
    ),
    (
        47,
        &[
            // Rule80A A is OrderCapacity A (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("A"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 528,
                    text: "A",
                }],
            },
            // Rule80A B is OrderCapacity A (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("B"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 528,
                    text: "A",
                }],
            },
            // Rule80A C is OrderCapacity P with OrderRestrictions 1 3 (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("C"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 528,
                        text: "P",
                    },
                    Fill::Constant {
                        tag: 529,
                        text: "1 3",
                    },
                ],
            },
            // Rule80A D is OrderCapacity P with OrderRestrictions 1 2 (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("D"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 528,
                        text: "P",
                    },
                    Fill::Constant {
                        tag: 529,
                        text: "1 2",
                    },
                ],
            },
            // Rule80A E is OrderCapacity P (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("E"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 528,
                    text: "P",
                }],
            },
            // Rule80A F is OrderCapacity W (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("F"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 528,
                    text: "W",
                }],
            },
            // Rule80A H is OrderCapacity I (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("H"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 528,
                    text: "I",
                }],
            },
            // Rule80A I is OrderCapacity I (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("I"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 528,
                    text: "I",
                }],
            },
            // Rule80A J is OrderCapacity I with OrderRestrictions 1 2 (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("J"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 528,
                        text: "I",
                    },
                    Fill::Constant {
                        tag: 529,
                        text: "1 2",
                    },
                ],
            },
            // Rule80A K is OrderCapacity I with OrderRestrictions 1 3 (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("K"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 528,
                        text: "I",
                    },
                    Fill::Constant {
                        tag: 529,
                        text: "1 3",
                    },
                ],
            },
            // Rule80A L is OrderCapacity P with OrderRestrictions 4 (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("L"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 528,
                        text: "P",
                    },
                    Fill::Constant {
                        tag: 529,
                        text: "4",
                    },
                ],
            },
            // Rule80A M is OrderCapacity W with OrderRestrictions 1 2 (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("M"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 528,
                        text: "W",
                    },
                    Fill::Constant {
                        tag: 529,
                        text: "1 2",
                    },
                ],
            },
            // Rule80A N is OrderCapacity W with OrderRestrictions 1 3 (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("N"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 528,
                        text: "W",
                    },
                    Fill::Constant {
                        tag: 529,
                        text: "1 3",
                    },
                ],
            },
            // Rule80A O is OrderCapacity P with OrderRestrictions 4 (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("O"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 528,
                        text: "P",
                    },
                    Fill::Constant {
                        tag: 529,
                        text: "4",
                    },
                ],
            },
            // Rule80A P is OrderCapacity P (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("P"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 528,
                    text: "P",
                }],
            },
            // Rule80A R is OrderCapacity A with OrderRestrictions 4 (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("R"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 528,
                        text: "A",
                    },
                    Fill::Constant {
                        tag: 529,
                        text: "4",
                    },
                ],
            },
            // Rule80A S is OrderCapacity P with OrderRestrictions 5 (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("S"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 528,
                        text: "P",
                    },
                    Fill::Constant {
                        tag: 529,
                        text: "5",
                    },
                ],
            },
            // Rule80A T is OrderCapacity W with OrderRestrictions 5 (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("T"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 528,
                        text: "W",
                    },
                    Fill::Constant {
                        tag: 529,
                        text: "5",
                    },
                ],
            },
            // Rule80A U is OrderCapacity A with OrderRestrictions 1 2 (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("U"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 528,
                        text: "A",
                    },
                    Fill::Constant {
                        tag: 529,
                        text: "1 2",
                    },
                ],
            },
            // Rule80A W is OrderCapacity W (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("W"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 528,
                    text: "W",
                }],
            },
            // Rule80A X is OrderCapacity W with OrderRestrictions 4 (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("X"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 528,
                        text: "W",
                    },
                    Fill::Constant {
                        tag: 529,
                        text: "4",
                    },
                ],
            },
            // Rule80A Y is OrderCapacity A with OrderRestrictions 1 3 (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("Y"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 528,
                        text: "A",
                    },
                    Fill::Constant {
                        tag: 529,
                        text: "1 3",
                    },
                ],
            },
            // Rule80A Z is OrderCapacity A with OrderRestrictions 4 (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("Z"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 528,
                        text: "A",
                    },
                    Fill::Constant {
                        tag: 529,
                        text: "4",
                    },
                ],
            },
        ],
    ),
    (
        63,
        &[
            // SettlType T+1 is NextDay (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("A"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant { tag: 63, text: "2" }],
            },
        ],
    ),
    (
        71,
        &[
            // A New allocation is AllocType Calculated (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("0"),
                msgtypes: &["J"],
                within: None,
                fills: &[Fill::Constant {
                    tag: 626,
                    text: "1",
                }],
            },
            // A Preliminary allocation is a New one of AllocType Preliminary (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("3"),
                msgtypes: &["J"],
                within: None,
                fills: &[
                    Fill::Constant { tag: 71, text: "0" },
                    Fill::Constant {
                        tag: 626,
                        text: "2",
                    },
                ],
            },
        ],
    ),
    (
        76,
        &[
            // ExecBroker is a party with PartyRole ExecutingFirm (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Any,
                msgtypes: &[],
                within: None,
                fills: &[Fill::Occurrence {
                    group: "parties",
                    members: &[
                        Fill::Source { tag: 448 },
                        Fill::Constant {
                            tag: 452,
                            text: "1",
                        },
                    ],
                }],
            },
        ],
    ),
    (
        92,
        &[
            // BrokerOfCredit is a party with PartyRole BrokerOfCredit (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Any,
                msgtypes: &[],
                within: None,
                fills: &[Fill::Occurrence {
                    group: "parties",
                    members: &[
                        Fill::Source { tag: 448 },
                        Fill::Constant {
                            tag: 452,
                            text: "2",
                        },
                    ],
                }],
            },
        ],
    ),
    (
        109,
        &[
            // ClientID is a party with PartyRole ClientID (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Any,
                msgtypes: &[],
                within: None,
                fills: &[Fill::Occurrence {
                    group: "parties",
                    members: &[
                        Fill::Source { tag: 448 },
                        Fill::Constant {
                            tag: 452,
                            text: "3",
                        },
                    ],
                }],
            },
        ],
    ),
    (
        111,
        &[Rule {
            when: When::Any,
            msgtypes: &[],
            within: None,
            fills: &[Fill::Source { tag: 1138 }],
        }],
    ),
    (
        119,
        &[Rule {
            when: When::Any,
            msgtypes: &[],
            within: Some("allocgrp"),
            fills: &[Fill::Source { tag: 737 }],
        }],
    ),
    (
        120,
        &[Rule {
            when: When::Any,
            msgtypes: &[],
            within: Some("allocgrp"),
            fills: &[Fill::Source { tag: 736 }],
        }],
    ),
    (
        150,
        &[
            // ExecType PartiallyFilled is ExecType Trade (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("1"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 150,
                    text: "F",
                }],
            },
            // ExecType Filled is ExecType Trade (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("2"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 150,
                    text: "F",
                }],
            },
        ],
    ),
    (
        166,
        &[
            // SettlLocation is a SettlementLocation party with a market participant identifier (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("CED"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Occurrence {
                    group: "parties",
                    members: &[
                        Fill::Source { tag: 448 },
                        Fill::Constant {
                            tag: 447,
                            text: "C",
                        },
                        Fill::Constant {
                            tag: 452,
                            text: "10",
                        },
                    ],
                }],
            },
            // SettlLocation is a SettlementLocation party with a market participant identifier (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("DTC"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Occurrence {
                    group: "parties",
                    members: &[
                        Fill::Source { tag: 448 },
                        Fill::Constant {
                            tag: 447,
                            text: "C",
                        },
                        Fill::Constant {
                            tag: 452,
                            text: "10",
                        },
                    ],
                }],
            },
            // SettlLocation is a SettlementLocation party with a market participant identifier (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("EUR"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Occurrence {
                    group: "parties",
                    members: &[
                        Fill::Source { tag: 448 },
                        Fill::Constant {
                            tag: 447,
                            text: "C",
                        },
                        Fill::Constant {
                            tag: 452,
                            text: "10",
                        },
                    ],
                }],
            },
            // SettlLocation is a SettlementLocation party with a market participant identifier (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("FED"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Occurrence {
                    group: "parties",
                    members: &[
                        Fill::Source { tag: 448 },
                        Fill::Constant {
                            tag: 447,
                            text: "C",
                        },
                        Fill::Constant {
                            tag: 452,
                            text: "10",
                        },
                    ],
                }],
            },
            // SettlLocation is a SettlementLocation party with a market participant identifier (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("PNY"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Occurrence {
                    group: "parties",
                    members: &[
                        Fill::Source { tag: 448 },
                        Fill::Constant {
                            tag: 447,
                            text: "C",
                        },
                        Fill::Constant {
                            tag: 452,
                            text: "10",
                        },
                    ],
                }],
            },
            // SettlLocation is a SettlementLocation party with a market participant identifier (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("PTC"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Occurrence {
                    group: "parties",
                    members: &[
                        Fill::Source { tag: 448 },
                        Fill::Constant {
                            tag: 447,
                            text: "C",
                        },
                        Fill::Constant {
                            tag: 452,
                            text: "10",
                        },
                    ],
                }],
            },
            // SettlLocation is a SettlementLocation party identified by ISO country code (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Any,
                msgtypes: &[],
                within: None,
                fills: &[Fill::Occurrence {
                    group: "parties",
                    members: &[
                        Fill::Source { tag: 448 },
                        Fill::Constant {
                            tag: 447,
                            text: "E",
                        },
                        Fill::Constant {
                            tag: 452,
                            text: "10",
                        },
                    ],
                }],
            },
        ],
    ),
    (
        167,
        &[
            // SecurityType UST is TNOTE (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("UST"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 167,
                    text: "TNOTE",
                }],
            },
            // SecurityType USTB is TBILL (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("USTB"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 167,
                    text: "TBILL",
                }],
            },
        ],
    ),
    (
        198,
        &[Rule {
            when: When::Any,
            msgtypes: &["r"],
            within: None,
            fills: &[Fill::Source { tag: 1369 }],
        }],
    ),
    (
        204,
        &[
            // CustomerOrFirm Customer is OrderCapacity Agency (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("0"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 528,
                    text: "A",
                }],
            },
            // CustomerOrFirm Firm is OrderCapacity Principal (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Equals("1"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 528,
                    text: "P",
                }],
            },
        ],
    ),
    (
        205,
        &[
            // MaturityDay completes MaturityMonthYear into MaturityDate (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Any,
                msgtypes: &[],
                within: None,
                fills: &[Fill::Join {
                    tag: 541,
                    parts: &[Part::Text(200), Part::TwoDigits(205)],
                }],
            },
        ],
    ),
    (
        210,
        &[Rule {
            when: When::Any,
            msgtypes: &[],
            within: None,
            fills: &[Fill::Source { tag: 1082 }],
        }],
    ),
    (
        219,
        &[
            // Benchmark CURVE is the interpolated USD Treasury curve (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("1"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 220,
                        text: "USD",
                    },
                    Fill::Constant {
                        tag: 221,
                        text: "Treasury",
                    },
                    Fill::Constant {
                        tag: 222,
                        text: "INTERPOLATED",
                    },
                ],
            },
            // Benchmark 5YR is the USD Treasury 5Y point (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("2"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 220,
                        text: "USD",
                    },
                    Fill::Constant {
                        tag: 221,
                        text: "Treasury",
                    },
                    Fill::Constant {
                        tag: 222,
                        text: "5Y",
                    },
                ],
            },
            // Benchmark OLD5 is the USD Treasury 5Y-OLD point (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("3"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 220,
                        text: "USD",
                    },
                    Fill::Constant {
                        tag: 221,
                        text: "Treasury",
                    },
                    Fill::Constant {
                        tag: 222,
                        text: "5Y-OLD",
                    },
                ],
            },
            // Benchmark 10YR is the USD Treasury 10Y point (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("4"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 220,
                        text: "USD",
                    },
                    Fill::Constant {
                        tag: 221,
                        text: "Treasury",
                    },
                    Fill::Constant {
                        tag: 222,
                        text: "10Y",
                    },
                ],
            },
            // Benchmark OLD10 is the USD Treasury 10Y-OLD point (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("5"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 220,
                        text: "USD",
                    },
                    Fill::Constant {
                        tag: 221,
                        text: "Treasury",
                    },
                    Fill::Constant {
                        tag: 222,
                        text: "10Y-OLD",
                    },
                ],
            },
            // Benchmark 30YR is the USD Treasury 30Y point (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("6"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 220,
                        text: "USD",
                    },
                    Fill::Constant {
                        tag: 221,
                        text: "Treasury",
                    },
                    Fill::Constant {
                        tag: 222,
                        text: "30Y",
                    },
                ],
            },
            // Benchmark OLD30 is the USD Treasury 30Y-OLD point (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("7"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 220,
                        text: "USD",
                    },
                    Fill::Constant {
                        tag: 221,
                        text: "Treasury",
                    },
                    Fill::Constant {
                        tag: 222,
                        text: "30Y-OLD",
                    },
                ],
            },
            // Benchmark 3MOLIBOR is the USD LIBOR 3M point (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("8"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 220,
                        text: "USD",
                    },
                    Fill::Constant {
                        tag: 221,
                        text: "LIBOR",
                    },
                    Fill::Constant {
                        tag: 222,
                        text: "3M",
                    },
                ],
            },
            // Benchmark 6MOLIBOR is the USD LIBOR 6M point (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("9"),
                msgtypes: &[],
                within: None,
                fills: &[
                    Fill::Constant {
                        tag: 220,
                        text: "USD",
                    },
                    Fill::Constant {
                        tag: 221,
                        text: "LIBOR",
                    },
                    Fill::Constant {
                        tag: 222,
                        text: "6M",
                    },
                ],
            },
        ],
    ),
    (
        226,
        &[
            // A one-day RepurchaseTerm is TerminationType Overnight (FIX 4.4 Appendix 6-E)
            Rule {
                when: When::Equals("1"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 788,
                    text: "1",
                }],
            },
            // A longer RepurchaseTerm is TerminationType Term (FIX 4.4 Appendix 6-E)
            Rule {
                when: When::Any,
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 788,
                    text: "2",
                }],
            },
        ],
    ),
    (
        227,
        &[Rule {
            when: When::Any,
            msgtypes: &[],
            within: None,
            fills: &[Fill::Source { tag: 44 }],
        }],
    ),
    (
        239,
        &[Rule {
            when: When::Any,
            msgtypes: &[],
            within: None,
            fills: &[Fill::Source { tag: 310 }],
        }],
    ),
    (
        240,
        &[Rule {
            when: When::Any,
            msgtypes: &[],
            within: None,
            fills: &[Fill::Source { tag: 696 }],
        }],
    ),
    (
        310,
        &[
            // SecurityType UST is TNOTE (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("UST"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 310,
                    text: "TNOTE",
                }],
            },
            // SecurityType USTB is TBILL (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("USTB"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 310,
                    text: "TBILL",
                }],
            },
        ],
    ),
    (
        314,
        &[
            // UnderlyingMaturityDay completes UnderlyingMaturityMonthYear into UnderlyingMaturityDate (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Any,
                msgtypes: &[],
                within: None,
                fills: &[Fill::Join {
                    tag: 542,
                    parts: &[Part::Text(313), Part::TwoDigits(314)],
                }],
            },
        ],
    ),
    (
        370,
        &[
            // OnBehalfOfSendingTime is a hop stamped by OnBehalfOfCompID (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Any,
                msgtypes: &[],
                within: None,
                fills: &[Fill::Occurrence {
                    group: "hopgrp",
                    members: &[
                        Fill::Source { tag: 629 },
                        Fill::From {
                            tag: 628,
                            source: 115,
                        },
                    ],
                }],
            },
        ],
    ),
    (
        439,
        &[
            // ClearingFirm is a party with PartyRole ClearingFirm (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Any,
                msgtypes: &[],
                within: None,
                fills: &[Fill::Occurrence {
                    group: "parties",
                    members: &[
                        Fill::Source { tag: 448 },
                        Fill::Constant {
                            tag: 452,
                            text: "4",
                        },
                    ],
                }],
            },
        ],
    ),
    (
        440,
        &[
            // ClearingAccount is a PartySubID of the ClearingFirm party (FIX 4.3 Appendix 6-F)
            Rule {
                when: When::Any,
                msgtypes: &[],
                within: None,
                fills: &[Fill::Occurrence {
                    group: "parties",
                    members: &[
                        Fill::Constant {
                            tag: 452,
                            text: "4",
                        },
                        Fill::Occurrence {
                            group: "ptyssubgrp",
                            members: &[Fill::Source { tag: 523 }],
                        },
                    ],
                }],
            },
        ],
    ),
    (
        465,
        &[
            // QuantityType CONTRACTS is QtyType Contracts (FIX 4.4 Appendix 6-E)
            Rule {
                when: When::Equals("6"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 854,
                    text: "1",
                }],
            },
            // QuantityType SHARES, CURRENCY and PAR are QtyType Units (FIX 4.4 Appendix 6-E)
            Rule {
                when: When::Equals("1"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 854,
                    text: "0",
                }],
            },
            // QuantityType SHARES, CURRENCY and PAR are QtyType Units (FIX 4.4 Appendix 6-E)
            Rule {
                when: When::Equals("5"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 854,
                    text: "0",
                }],
            },
            // QuantityType SHARES, CURRENCY and PAR are QtyType Units (FIX 4.4 Appendix 6-E)
            Rule {
                when: When::Equals("8"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 854,
                    text: "0",
                }],
            },
        ],
    ),
    (
        540,
        &[Rule {
            when: When::Any,
            msgtypes: &[],
            within: None,
            fills: &[Fill::Source { tag: 159 }],
        }],
    ),
    (
        575,
        &[
            // An OddLot is LotType OddLot (FIX 5.0 Appendix 6-E)
            Rule {
                when: When::Equals("Y"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 1093,
                    text: "1",
                }],
            },
        ],
    ),
    (
        609,
        &[
            // SecurityType UST is TNOTE (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("UST"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 609,
                    text: "TNOTE",
                }],
            },
            // SecurityType USTB is TBILL (FIX 4.4 Appendix 6-F)
            Rule {
                when: When::Equals("USTB"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 609,
                    text: "TBILL",
                }],
            },
        ],
    ),
    (
        687,
        &[
            Rule {
                when: When::Any,
                msgtypes: &["R", "AJ", "AG", "S", "AI", "AB", "8"],
                within: None,
                fills: &[Fill::Source { tag: 685 }],
            },
            Rule {
                when: When::Any,
                msgtypes: &["AE", "AR"],
                within: None,
                fills: &[Fill::Source { tag: 1418 }],
            },
        ],
    ),
    (
        852,
        &[
            // PublishTrdIndicator Y is TradePublishIndicator PublishTrade (FIX 5.0 SP1 Appendix 6-E)
            Rule {
                when: When::Equals("Y"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 1390,
                    text: "1",
                }],
            },
            // PublishTrdIndicator N is TradePublishIndicator DoNotPublishTrade (FIX 5.0 SP1 Appendix 6-E)
            Rule {
                when: When::Equals("N"),
                msgtypes: &[],
                within: None,
                fills: &[Fill::Constant {
                    tag: 1390,
                    text: "0",
                }],
            },
        ],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_by_tag_with_each_tag_once() {
        for pair in RULES.windows(2) {
            assert!(pair[0].0 < pair[1].0, "{} before {}", pair[0].0, pair[1].0);
        }
        assert_eq!(RULES.len(), 37);
        assert_eq!(
            RULES.iter().map(|(_, rules)| rules.len()).sum::<usize>(),
            100
        );
    }

    #[test]
    fn a_lookup_answers_the_tag_it_is_asked_for() {
        assert!(rules_of(47).is_some_and(|rules| rules.len() == 23));
        assert!(rules_of(18).is_some_and(|rules| rules.len() == 9));
        assert!(rules_of(687).is_some_and(|rules| rules.len() == 2));
        assert!(rules_of(1).is_none());
        assert!(rules_of(9_999).is_none());
    }

    #[test]
    fn every_rule_fills_something_and_a_catch_all_comes_last() {
        for (tag, rules) in RULES {
            for (at, rule) in rules.iter().enumerate() {
                assert!(!rule.fills.is_empty(), "tag {tag} entry {at} fills nothing");
                if matches!(rule.when, When::Any)
                    && rule.msgtypes.is_empty()
                    && rule.within.is_none()
                {
                    assert_eq!(
                        at + 1,
                        rules.len(),
                        "tag {tag}: a catch-all is the last entry"
                    );
                }
            }
        }
    }
}
