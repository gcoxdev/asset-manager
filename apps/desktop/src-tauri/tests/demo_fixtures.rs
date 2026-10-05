//! Records a realistic demo catalog for the README screenshots.
//!
//! A household's worth of things — bullion, crypto, watches, a ring, art,
//! wine, electronics, a car, collectibles — each bought on a date, valued
//! over time, some sold or lent, built through the real commands so the
//! screenshots show what the app really produces. Dates are relative to
//! today, so the charts and badges always look current.
//!
//! Writes only when `UI_DEMO_DIR` is set (`npm run screenshots`).

mod common;

use std::collections::BTreeMap;

use common::Recorder;
use serde_json::{json, Value};

/// `days` ago (negative: in the future), on the local calendar.
fn ago(days: i64) -> String {
    (chrono::Local::now().date_naive() - chrono::Duration::days(days))
        .format("%Y-%m-%d")
        .to_string()
}

struct Item {
    type_id: &'static str,
    name: &'static str,
    quantity: &'static str,
    unit: &'static str,
    bought: i64,
    paid: Option<&'static str>,
    from: &'static str,
    location: &'static str,
    tags: &'static [&'static str],
    attrs: Value,
    market: bool,
    insured: Option<&'static str>,
    review: Option<i64>,
    /// (days ago, value of the whole holding)
    values: &'static [(i64, &'static str)],
}

fn items() -> Vec<Item> {
    vec![
        Item {
            type_id: "sovereign_coin",
            name: "American Gold Eagle 1 oz",
            quantity: "10",
            unit: "coins",
            bought: 1310,
            paid: Some("18240"),
            from: "APMEX",
            location: "Safe / Top shelf",
            tags: &["bullion"],
            attrs: json!({ "metal": "XAU", "weight_per_item": "1.0909", "weight_unit": "troy_oz",
                           "weight_basis": "gross", "purity": "0.9167", "preset": "age" }),
            market: true,
            insured: Some("30000"),
            review: None,
            values: &[
                (720, "25800"),
                (540, "27300"),
                (365, "29400"),
                (270, "31600"),
                (180, "32900"),
                (90, "33800"),
            ],
        },
        Item {
            type_id: "sovereign_coin",
            name: "Silver Maple Leaf 1 oz",
            quantity: "250",
            unit: "coins",
            bought: 1500,
            paid: Some("6950"),
            from: "Local coin shop",
            location: "Safe / Bottom drawer",
            tags: &["bullion"],
            attrs: json!({ "metal": "XAG", "weight_per_item": "1", "weight_unit": "troy_oz",
                           "weight_basis": "fine", "purity": "0.9999", "preset": "maple_silver" }),
            market: true,
            insured: None,
            review: None,
            values: &[
                (720, "7400"),
                (540, "7700"),
                (365, "8150"),
                (270, "8900"),
                (180, "9500"),
                (90, "9900"),
            ],
        },
        Item {
            type_id: "gold_bullion",
            name: "PAMP Suisse 1 oz gold bar",
            quantity: "2",
            unit: "bars",
            bought: 200,
            paid: Some("6240"),
            from: "JM Bullion",
            location: "Safe / Top shelf",
            tags: &["bullion"],
            attrs: json!({ "metal": "XAU", "weight_per_item": "1", "weight_unit": "troy_oz",
                           "weight_basis": "fine", "purity": "0.9999", "preset": "gold_bar_1oz" }),
            market: true,
            insured: None,
            review: None,
            values: &[(180, "6600"), (90, "6750")],
        },
        Item {
            type_id: "crypto",
            name: "Bitcoin — cold storage",
            quantity: "0.85",
            unit: "BTC",
            bought: 1100,
            paid: Some("19550"),
            from: "Coinbase",
            location: "",
            tags: &[],
            attrs: json!({ "coin_id": "bitcoin", "symbol": "BTC", "custody": "self_custody" }),
            market: true,
            insured: None,
            review: None,
            values: &[
                (720, "47800"),
                (540, "58200"),
                (365, "61900"),
                (270, "79500"),
                (180, "88400"),
                (90, "96300"),
            ],
        },
        Item {
            type_id: "crypto",
            name: "Ethereum",
            quantity: "6.4",
            unit: "ETH",
            bought: 900,
            paid: Some("9800"),
            from: "Kraken",
            location: "",
            tags: &[],
            attrs: json!({ "coin_id": "ethereum", "symbol": "ETH", "custody": "exchange", "wallet": "Kraken" }),
            market: true,
            insured: None,
            review: None,
            values: &[
                (720, "16200"),
                (540, "21500"),
                (365, "16900"),
                (270, "11800"),
                (180, "17500"),
                (90, "24600"),
            ],
        },
        Item {
            type_id: "security",
            name: "Vanguard Total Stock Market ETF",
            quantity: "45",
            unit: "shares",
            bought: 1000,
            paid: Some("9450"),
            from: "",
            location: "",
            tags: &[],
            attrs: json!({ "ticker": "VTI", "issuer": "Vanguard", "held_at": "Brokerage account" }),
            market: false,
            insured: None,
            review: Some(90),
            values: &[
                (720, "10300"),
                (540, "11600"),
                (365, "12400"),
                (270, "11900"),
                (180, "13200"),
                (60, "13800"),
            ],
        },
        Item {
            type_id: "watch",
            name: "Rolex Submariner Date 126610LN",
            quantity: "1",
            unit: "item",
            bought: 840,
            paid: Some("10250"),
            from: "Authorized dealer",
            location: "Bedroom / Watch box",
            tags: &["insured", "daily wear"],
            attrs: json!({ "brand": "Rolex", "model": "Submariner Date", "reference": "126610LN",
                           "serial_number": "7Q2K8851", "year": "2022", "box_papers": "Full set" }),
            market: false,
            insured: Some("15000"),
            review: Some(365),
            values: &[(720, "12800"), (365, "13400"), (120, "14600")],
        },
        Item {
            type_id: "watch",
            name: "Omega Speedmaster Professional",
            quantity: "1",
            unit: "item",
            bought: 1900,
            paid: Some("5950"),
            from: "Omega boutique",
            location: "Bedroom / Watch box",
            tags: &["insured"],
            attrs: json!({ "brand": "Omega", "model": "Speedmaster Professional", "reference": "310.30.42.50.01.001",
                           "serial_number": "81447302", "year": "2020", "box_papers": "Full set" }),
            market: false,
            insured: Some("6500"),
            review: None,
            values: &[(500, "6100"), (200, "6300")],
        },
        Item {
            type_id: "jewelry",
            name: "Diamond engagement ring",
            quantity: "1",
            unit: "item",
            bought: 2400,
            paid: Some("11800"),
            from: "Brilliant & Co. Jewelers",
            location: "Bedroom / Jewelry box",
            tags: &["insured"],
            attrs: json!({ "metal_type": "Platinum", "stones": "1.52 ct round brilliant, F / VS1",
                           "carat": "1.52", "appraised_by": "Independent GIA graduate gemologist" }),
            market: false,
            insured: Some("14500"),
            review: None,
            values: &[(600, "14500")],
        },
        Item {
            type_id: "art",
            name: "Harbor at Dusk — oil on canvas",
            quantity: "1",
            unit: "item",
            bought: 1000,
            paid: Some("3200"),
            from: "Gallery on Fifth",
            location: "Living room",
            tags: &[],
            attrs: json!({ "artist": "Mara Okafor", "title": "Harbor at Dusk", "medium": "Oil on canvas",
                           "dimensions": "24 × 36 in" }),
            market: false,
            insured: None,
            review: None,
            values: &[(300, "3600")],
        },
        Item {
            type_id: "instrument",
            name: "Fender American Professional II Stratocaster",
            quantity: "1",
            unit: "item",
            bought: 760,
            paid: Some("1700"),
            from: "Guitar Center",
            location: "Music room",
            tags: &[],
            attrs: json!({ "maker": "Fender", "model": "American Professional II Stratocaster",
                           "serial_number": "US22041871", "year": "2022", "condition": "Excellent" }),
            market: false,
            insured: None,
            review: Some(180),
            values: &[(365, "1450")],
        },
        Item {
            type_id: "wine",
            name: "Château Margaux 2015",
            quantity: "6",
            unit: "bottles",
            bought: 1600,
            paid: Some("4200"),
            from: "K&L Wine Merchants",
            location: "Basement / Wine rack",
            tags: &[],
            attrs: json!({ "producer": "Château Margaux", "vintage": "2015", "bottle_size": "750 ml",
                           "storage_conditions": "55°F, 70% humidity" }),
            market: false,
            insured: None,
            review: None,
            values: &[(720, "5100"), (365, "5500"), (90, "5900")],
        },
        Item {
            type_id: "electronics",
            name: "MacBook Pro 16-inch",
            quantity: "1",
            unit: "item",
            bought: 330,
            paid: Some("3199"),
            from: "Apple Store",
            location: "Office",
            tags: &[],
            attrs: json!({ "brand": "Apple", "model": "MacBook Pro 16-inch, M4 Pro", "serial_number": "C02XK1LMQ6L4" }),
            market: false,
            insured: None,
            review: None,
            values: &[(300, "2900"), (60, "2450")],
        },
        Item {
            type_id: "electronics",
            name: "Sony Alpha 7 IV camera kit",
            quantity: "1",
            unit: "item",
            bought: 640,
            paid: Some("2800"),
            from: "B&H Photo",
            location: "Office",
            tags: &[],
            attrs: json!({ "brand": "Sony", "model": "Alpha 7 IV with 24-70mm f/2.8 GM II",
                           "serial_number": "5291847" }),
            market: false,
            insured: None,
            review: None,
            values: &[(365, "2200"), (100, "1900")],
        },
        Item {
            type_id: "vehicle",
            name: "2021 Toyota 4Runner TRD Off-Road",
            quantity: "1",
            unit: "item",
            bought: 1500,
            paid: Some("45900"),
            from: "Dealer",
            location: "Garage",
            tags: &["insured"],
            attrs: json!({ "make": "Toyota", "model": "4Runner TRD Off-Road", "year": "2021",
                           "vin": "JTEBU5JR0M5000123", "color": "Army Green", "mileage": "41200" }),
            market: false,
            insured: Some("40000"),
            review: None,
            values: &[(720, "41000"), (365, "39500"), (30, "37800")],
        },
        Item {
            type_id: "comic",
            name: "",
            quantity: "1",
            unit: "item",
            bought: 1200,
            paid: Some("3900"),
            from: "Heritage Auctions",
            location: "Office / Comic box",
            tags: &[],
            attrs: json!({ "title": "The Amazing Spider-Man", "issue": "300", "grader": "cgc",
                           "grade": "9.8", "cert_number": "3912447002" }),
            market: false,
            insured: None,
            review: None,
            values: &[(720, "5200"), (365, "5800"), (150, "6400")],
        },
        Item {
            type_id: "trading_card",
            name: "",
            quantity: "1",
            unit: "item",
            bought: 900,
            paid: Some("1100"),
            from: "eBay",
            location: "Office / Card case",
            tags: &[],
            attrs: json!({ "player_or_character": "Shohei Ohtani", "set": "2018 Topps Update",
                           "year": "2018", "card_number": "US1", "grader": "psa", "grade": "10",
                           "cert_number": "61234558" }),
            market: false,
            insured: None,
            review: None,
            values: &[(365, "1350"), (120, "1580")],
        },
        Item {
            type_id: "tcg_card",
            name: "",
            quantity: "1",
            unit: "item",
            bought: 1300,
            paid: Some("2100"),
            from: "Card show",
            location: "Office / Card case",
            tags: &[],
            attrs: json!({ "name": "Charizard", "set": "Base Set", "grader": "psa", "grade": "8",
                           "cert_number": "48812205" }),
            market: false,
            insured: None,
            review: None,
            values: &[(720, "3900"), (365, "4300"), (90, "4800")],
        },
        Item {
            type_id: "numismatic_coin",
            name: "",
            quantity: "1",
            unit: "item",
            bought: 2000,
            paid: Some("1350"),
            from: "Coin show",
            location: "Safe / Top shelf",
            tags: &[],
            attrs: json!({ "denomination": "1c", "year": "1909", "mint_mark": "S", "variety": "VDB",
                           "grader": "pcgs", "grade": "30", "cert_number": "40551872" }),
            market: false,
            insured: None,
            review: None,
            values: &[(500, "1750"), (200, "1850")],
        },
        Item {
            type_id: "toy",
            name: "LEGO Star Wars Millennium Falcon 75192",
            quantity: "1",
            unit: "item",
            bought: 1700,
            paid: Some("799"),
            from: "LEGO Store",
            location: "Basement / Shelves",
            tags: &[],
            attrs: json!({ "brand": "LEGO", "line": "Star Wars UCS", "set_number": "75192",
                           "year": "2017", "packaging": "Sealed" }),
            market: false,
            insured: None,
            review: None,
            values: &[(365, "950"), (90, "1050")],
        },
        Item {
            type_id: "watch",
            name: "Grandfather's Elgin pocket watch",
            quantity: "1",
            unit: "item",
            bought: 12,
            paid: None,
            from: "Inherited",
            location: "Bedroom / Jewelry box",
            tags: &["family"],
            attrs: json!({ "brand": "Elgin", "year": "1912" }),
            market: false,
            insured: None,
            review: None,
            values: &[],
        },
    ]
}

#[test]
fn record_demo_fixtures() {
    let Some(out) = std::env::var_os("UI_DEMO_DIR") else {
        eprintln!("UI_DEMO_DIR not set; nothing recorded");
        return;
    };
    std::fs::create_dir_all(&out).unwrap();
    let out = std::fs::canonicalize(&out).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let (_app, w) = common::app(&dir.path().join("vault"));
    let mut r = Recorder { w: &w, calls: BTreeMap::new() };

    r.run("create_vault", json!({ "passphrase": "correct horse battery staple" }));
    r.run("confirm_recovery_saved", json!({}));

    let mut ids = BTreeMap::new();
    for item in items() {
        let id = r.run(
            "create_asset",
            json!({ "form": {
                "type_id": item.type_id,
                "name": if item.name.is_empty() { Value::Null } else { json!(item.name) },
                "quantity": item.quantity, "quantity_unit": item.unit,
                "acquired_date": ago(item.bought), "acquired_price": item.paid,
                "acquired_from": item.from, "storage_location": item.location,
                "notes": "", "currency": "USD", "insured_value": item.insured,
                "review_every_days": item.review, "attrs": item.attrs,
                "pricing": if item.market { "market" } else { "manual" },
            }}),
        );
        let id = id.as_str().unwrap().to_string();
        if !item.tags.is_empty() {
            r.run("set_asset_tags", json!({ "assetId": id, "tags": item.tags }));
        }
        let ring = item.type_id == "jewelry";
        // Worth what was paid on the day it was bought, then as recorded.
        let opening =
            item.paid.filter(|_| item.values.first().is_none_or(|(d, _)| *d < item.bought));
        let entries: Vec<Value> = opening
            .map(|p| (item.bought, p))
            .into_iter()
            .chain(item.values.iter().copied())
            .map(|(days, value)| {
                json!({ "asset_id": id, "amount": value, "currency": "USD", "asof": ago(days),
                        "basis": if ring { "replacement" } else { "estimated_resale" },
                        "provenance": if ring { "appraisal" } else { "manual" }, "note": null })
            })
            .collect();
        if !entries.is_empty() {
            let results = r.run("set_prices", json!({ "entries": entries }));
            assert!(results.as_array().unwrap().iter().all(|x| x["ok"] == true), "{results}");
        }
        // Typed values switch a holding to manual; put market ones back.
        if item.market {
            r.run("set_pricing", json!({ "assetId": id, "pricing": "market" }));
        }
        ids.insert(item.name.to_string(), id);
    }
    let id = |name: &str| ids[name].clone();

    // What happened along the way.
    r.run(
        "change_quantity",
        json!({ "change": { "asset_id": id("Silver Maple Leaf 1 oz"), "kind": "remove", "quantity": "50",
                            "effective_date": ago(40), "amount": "2050", "currency": "USD",
                            "note": "Sold to the coin shop" } }),
    );
    r.run(
        "change_quantity",
        json!({ "change": { "asset_id": id("Vanguard Total Stock Market ETF"), "kind": "add", "quantity": "10",
                            "effective_date": ago(25), "amount": "3080", "currency": "USD", "note": null } }),
    );
    r.run(
        "record_custody",
        json!({ "entry": { "asset_id": id("Sony Alpha 7 IV camera kit"), "kind": "lent",
                           "party": "Jordan Reyes", "date": ago(4), "due_back": ago(-10),
                           "note": "For a wedding shoot" } }),
    );
    r.run(
        "add_care",
        json!({ "entry": { "asset_id": id("Omega Speedmaster Professional"), "kind": "service",
                           "performed_on": ago(1460), "provider": "Omega Service Center",
                           "cost": "750", "currency": "USD", "next_due": ago(-40) } }),
    );
    r.run(
        "add_care",
        json!({ "entry": { "asset_id": id("MacBook Pro 16-inch"), "kind": "warranty",
                           "provider": "AppleCare+", "next_due": ago(-35) } }),
    );

    // Today's market prices revalue the market-priced holdings.
    for (metal, price) in [("XAU", "3480"), ("XAG", "41.20"), ("XPT", "1190"), ("XPD", "1060")]
    {
        r.run("set_spot_price", json!({ "metal": metal, "price": price, "currency": "USD" }));
    }
    r.run("set_coin_price", json!({ "coinId": "bitcoin", "price": "112000" }));
    r.run("set_coin_price", json!({ "coinId": "ethereum", "price": "3900" }));

    // A fresh backup, as a careful owner would have.
    let backups = dir.path().join("backups");
    std::fs::create_dir_all(&backups).unwrap();
    r.run("backup_vault", json!({ "directory": backups.to_str().unwrap() }));

    // --- what the screens ask for --------------------------------------------
    for cmd in [
        "vault_status",
        "session_state",
        "get_settings",
        "list_assets",
        "asset_types",
        "collectible_types",
        "graders",
        "bullion_presets",
        "common_coins",
        "list_custom_types",
        "dashboard",
        "away_list",
        "list_tags",
        "list_locations",
        "list_saved_views",
        "list_sets",
        "spot_prices",
        "metals_provider_status",
        "crypto_prices",
        "crypto_provider_status",
        "list_rates",
    ] {
        r.record(cmd, json!({}));
    }
    r.record("care_due", json!({ "withinDays": 60 }));
    r.record("portfolio_series", json!({ "from": ago(365), "maxPoints": 160 }));
    let rolex = id("Rolex Submariner Date 126610LN");
    for cmd in ["get_asset", "list_photos", "list_care", "list_custody", "asset_revisions"] {
        r.record(cmd, json!({ "assetId": rolex }));
    }

    common::save(&out.join("demo.json"), json!({ "rolex": rolex }), &r.calls);
}
