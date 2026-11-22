use super::*;

const ALL: [StatsSection; 7] = [
    StatsSection::Overview,
    StatsSection::Cameras,
    StatsSection::Lenses,
    StatsSection::Exposure,
    StatsSection::Time,
    StatsSection::Places,
    StatsSection::Top,
];

fn lib() -> Library {
    Library { roots: vec!["/lib".into()], hidden: vec!["/lib/hidden".into()] }
}
fn photo(path: &str) -> Parsed {
    Parsed { path: path.into(), ..Default::default() }
}
fn at(s: &str) -> Option<i64> {
    parse_local(s)
}
fn run(parsed: Vec<Parsed>, scope: StatsScope, filter: StatsFilter) -> Sections {
    let idx = Index::build(parsed, &[]);
    run_on(&idx, &scope, &filter, None)
}
fn run_on(idx: &Index, scope: &StatsScope, filter: &StatsFilter, view: Option<PlacesView>) -> Sections {
    let scoped = resolve_scope(idx, scope, &lib()).unwrap();
    let q = Query::new(idx, &scoped, filter);
    let id = |p: &str| p.to_string();
    stats_snapshot(&q, &Photos { index: idx, id: &id }, view)
}
fn stats_snapshot(q: &Query, p: &Photos, view: Option<PlacesView>) -> Sections {
    snapshot(q, p, &Wanted { sections: &ALL, places: view })
}
fn f() -> StatsFilter {
    StatsFilter { min_rating: 4, ..Default::default() }
}
fn canon(path: &str, lens: &str, rating: u8) -> Parsed {
    Parsed {
        path: path.into(),
        maker: "Canon".into(),
        model: "EOS R5".into(),
        lens: lens.into(),
        rating,
        ..Default::default()
    }
}

#[test]
fn focal_buckets_follow_the_contract_edges() {
    let b = |mm| focal_bucket(mm);
    assert_eq!([b(0), b(1), b(20), b(21)], [UNKNOWN, 0, 0, 1]);
    assert_eq!([b(34), b(35), b(69), b(70)], [1, 2, 2, 3]);
    assert_eq!([b(135), b(136), b(300), b(301)], [3, 4, 4, 5]);
}

#[test]
fn aperture_groups_include_their_upper_edge() {
    let g = |v: f32| aperture_group(v);
    assert_eq!(g(0.0), UNKNOWN);
    assert_eq!([g(1.2), g(1.8), g(2.0), g(2.8)], [0, 0, 1, 1]);
    assert_eq!([g(4.0), g(5.6), g(8.0), g(11.0)], [2, 2, 3, 3]);
    assert_eq!([g(16.0), g(22.0)], [4, 4]);
}

#[test]
fn shutter_regimes_are_half_open_at_the_stated_edges() {
    let r = |t: f64| shutter_regime(t as f32);
    assert_eq!(r(0.0), UNKNOWN);
    assert_eq!([r(1.0 / 8000.0), r(0.001), r(1.0 / 751.0)], [0, 0, 0]);
    // just past the edge is the slower regime (the f32 a cache holds cannot land on 1/750 exactly)
    assert_eq!([r(1.0 / 749.0), r(0.002), r(1.0 / 250.0), r(1.0 / 181.0)], [1, 1, 1, 1]);
    assert_eq!([r(1.0 / 180.0), r(1.0 / 60.0), r(0.0166), r(1.0 / 46.0)], [2, 2, 2, 2]);
    assert_eq!([r(1.0 / 45.0), r(1.0 / 15.0), r(1.0 / 7.0)], [3, 3, 3]);
    assert_eq!([r(1.0 / 6.0), r(0.5), r(1.0)], [4, 4, 4]);
    assert_eq!([r(1.001), r(30.0)], [5, 5]);
}

#[test]
fn iso_bins_are_the_stop_at_or_below() {
    let b = |v: f32| iso_bin(v);
    assert_eq!(b(0.0), UNKNOWN);
    assert_eq!([b(50.0), b(100.0), b(149.0), b(150.0)], [0, 0, 0, 1]);
    assert_eq!([b(299.0), b(300.0), b(599.0), b(600.0)], [1, 2, 2, 3]);
    assert_eq!([b(1199.0), b(1200.0), b(2399.0), b(2400.0)], [3, 4, 4, 5]);
    assert_eq!([b(4799.0), b(4800.0), b(102_400.0)], [5, 6, 6]);
}

#[test]
fn aspect_classes_use_the_long_over_short_ratio_within_two_percent() {
    assert_eq!(aspect_class(0, 10), UNKNOWN);
    // landscape and portrait of one shape are one class
    assert_eq!([aspect_class(6000, 4000), aspect_class(4000, 6000)], [1, 1]);
    assert_eq!(aspect_class(1920, 1080), 0);
    assert_eq!(aspect_class(4000, 3000), 2);
    assert_eq!(aspect_class(1000, 1000), 3);
    assert_eq!(aspect_class(1000, 1015), 3);
    assert_eq!(aspect_class(5000, 4000), 4);
    assert_eq!(aspect_class(3000, 1000), 6);
    // 2.0 itself is neither 16:9 nor a panorama
    assert_eq!(aspect_class(2000, 1000), 7);
    // just outside 3:2 (1.53) is other
    assert_eq!(aspect_class(1540, 1000), 7);
    assert_eq!(aspect_class(1530, 1000), 1);
}

#[test]
fn makers_and_lenses_are_normalised() {
    assert_eq!(norm_maker("SONY"), "Sony");
    assert_eq!(norm_maker("NIKON CORPORATION"), "Nikon");
    assert_eq!(norm_maker(" Canon "), "Canon");
    assert_eq!(norm_maker("ACME CORP"), "Acme Corp");
    assert_eq!(norm_maker("Leica Camera AG"), "Leica");
    assert_eq!(norm_maker(""), "");
    assert_eq!(norm_lens("  FE  24-70mm   F2.8 "), "FE 24-70mm F2.8");
    assert_eq!(body_label("Canon", "Canon EOS R5"), "Canon EOS R5");
    assert_eq!(body_label("Sony", "ILCE-7M4"), "Sony ILCE-7M4");
}

fn raw(meta: &str) -> RawRow {
    RawRow {
        variant_id: 1,
        path: "/lib/a.jpg".into(),
        capture_time_ns: None,
        capture_time_raw: None,
        timezone_offset_seconds: None,
        metadata: Some(meta.into()),
    }
}

#[test]
fn capture_time_is_the_camera_local_clock() {
    // the EXIF local time wins over the stored instant and its offset
    let mut r = raw(r#"{"exif":{"datetime":"2024:07:04 06:30:00"}}"#);
    r.capture_time_ns = Some(1_720_078_200_000_000_000); // 2024-07-04T07:30:00Z
    r.timezone_offset_seconds = Some(-8 * 3600);
    let p = parse_row(&r).unwrap();
    assert_eq!(p.taken, at("2024:07:04 06:30:00"));
    // only the instant and its offset: local = UTC + offset
    let mut r = raw(r#"{"exif":{}}"#);
    r.capture_time_ns = Some(1_720_078_200_000_000_000);
    r.timezone_offset_seconds = Some(2 * 3600);
    assert_eq!(parse_row(&r).unwrap().taken, at("2024:07:04 09:30:00"));
    // an instant with no known offset is not a local time
    r.timezone_offset_seconds = None;
    assert_eq!(parse_row(&r).unwrap().taken, None);
    // the raw column fills in for a missing EXIF time; fractions and dashes parse
    let mut r = raw(r#"{"exif":{}}"#);
    r.capture_time_raw = Some("2020:02:29 23:59:58.250".into());
    assert_eq!(parse_row(&r).unwrap().taken, at("2020-02-29 23:59:58"));
    // impossible dates are unknown, not clamped
    for bad in ["0000:00:00 00:00:00", "2023:02:30 10:00:00", "2023:13:01 10:00:00", "2023:01:01 25:00:00", ""] {
        let r = raw(&format!(r#"{{"exif":{{"datetime":"{bad}"}}}}"#));
        assert_eq!(parse_row(&r).unwrap().taken, None, "{bad}");
    }
    // a row with no metadata yet is not a photo
    let mut r = raw("{}");
    r.metadata = None;
    assert!(parse_row(&r).is_none());
}

#[test]
fn parse_row_cleans_fields() {
    let p = parse_row(&raw(
        r#"{"rating":3,"rejected":true,"labels":33,"width":6000,"height":4000,
            "location":{"latitude":0.0,"longitude":0.0},
            "exif":{"maker":"SONY","model":" ILCE-7M4 ","lens":"FE  85mm F1.8","exposure":0.004,
                    "aperture":1.8,"iso":400,"focal_length":50.0,"crop_factor":1.5,"future_field":1}}"#,
    ))
    .unwrap();
    // rejected photos have no stars; only the five label bits survive
    assert_eq!((p.rating, p.rejected, p.labels), (0, true, 1));
    assert_eq!((p.maker.as_str(), p.model.as_str(), p.lens.as_str()), ("Sony", "ILCE-7M4", "FE 85mm F1.8"));
    assert_eq!(p.focal, 75.0); // 35 mm equivalent
    assert_eq!(p.gps, None); // 0,0 is "no position"
    let p = parse_row(&raw(r#"{"location":{"latitude":47.6,"longitude":-122.3},"exif":{"focal_length":24.0}}"#)).unwrap();
    assert_eq!(p.focal, 24.0); // no crop factor: the real length
    assert_eq!(p.gps, Some((47.6, -122.3)));
}

#[test]
fn an_empty_library_answers_every_section() {
    let s = run(vec![], StatsScope::Library, f());
    let o = s.overview.unwrap();
    assert_eq!((o.photos, o.folders, o.first_year, o.rejected, o.edited), (0, 0, None, 0, 0));
    assert!(o.favourite_focal.is_none() && o.insights.is_empty() && o.peak.hour.is_none());
    assert!(o.median_setup.focal_mm.is_none());
    assert_eq!(o.rating_dist, [0; 6]);
    let c = s.cameras.unwrap();
    assert!((c.known, c.bodies.len(), c.years.len()) == (0, 0, 0));
    let e = s.exposure.unwrap();
    assert_eq!((e.aperture.len(), e.shutter.len(), e.iso.len()), (5, 6, 7));
    let t = s.time.unwrap();
    assert_eq!((t.hours.len(), t.months.len(), t.known), (24, 12, 0));
    let p = s.places.unwrap();
    assert_eq!((p.known, p.total, p.cells.len()), (0, 0, 0));
    assert!(p.bbox.is_none());
    let top = s.top.unwrap();
    assert_eq!((top.total, top.items.len()), (0, 0));
    assert!(s.lenses.unwrap().focal.modal_mm.is_none());
}

#[test]
fn a_single_photo_fills_each_block_once() {
    let one = Parsed {
        path: "/lib/2024/x.arw".into(),
        maker: "Sony".into(),
        model: "ILCE-7M4".into(),
        lens: "FE 85mm F1.8".into(),
        serial: "S1".into(),
        rating: 5,
        labels: 4,
        width: 6000,
        height: 4000,
        focal: 85.0,
        aperture: 1.8,
        shutter: 1.0 / 500.0,
        iso: 100.0,
        taken: at("2024:08:03 18:45:00"),
        gps: Some((47.6, -122.3)),
        ..Default::default()
    };
    let s = run(vec![one], StatsScope::Library, f());
    let o = s.overview.unwrap();
    assert_eq!((o.photos, o.matches.top, o.folders), (1, 1, 1));
    assert_eq!(o.rating_dist, [0, 0, 0, 0, 0, 1]);
    assert_eq!(o.labels.green, 1);
    assert_eq!((o.first_year, o.last_year), (Some(2024), Some(2024)));
    assert_eq!(o.gear.bodies + o.gear.lenses + o.gear.makers, 3);
    assert_eq!(o.favourite_focal.as_ref().unwrap().mm, 85);
    assert_eq!(o.median_setup.f_number, Some(1.7999999523162842));
    assert_eq!((o.peak.hour, o.peak.month), (Some(18), Some(8)));
    assert_eq!(o.file_types[0].kind, "ARW");
    // one photo is no comparison: no keeper-rate insights
    assert!(o.insights.iter().all(|i| i.id != InsightId::KeeperBody && i.id != InsightId::KeeperLens));
    let c = s.cameras.unwrap();
    assert_eq!(c.bodies[0].key, "Sony|ILCE-7M4");
    assert_eq!(c.bodies[0].cover.as_ref().unwrap().body.as_deref(), Some("Sony ILCE-7M4"));
    assert_eq!((c.bodies[0].serials, c.bodies[0].first_shot.as_deref()), (1, Some("2024-08-03")));
    assert_eq!(c.bodies[0].top_lens.as_deref(), Some("FE 85mm F1.8"));
    let l = s.lenses.unwrap();
    assert!(l.lenses[0].is_prime);
    assert_eq!(l.focal_by_bucket[3].n, 1);
    assert_eq!(l.focal_by_bucket[3].modal_mm, 85);
    let e = s.exposure.unwrap();
    assert_eq!((e.aperture[0].n, e.shutter[1].n, e.iso[0].n), (1, 1, 1));
    assert_eq!(e.aperture_by_shutter.unwrap()[0][1], 1);
    let t = s.time.unwrap();
    assert_eq!((t.hours[18].n, t.months[7].n, t.peak.hour, t.peak.month), (1, 1, 18, 8));
    let p = s.places.unwrap();
    assert_eq!((p.known, p.total, p.cells.len()), (1, 1, 1));
    assert!(p.cells[0].solo.is_some());
    assert_eq!(s.top.unwrap().items[0].path, "/lib/2024/x.arw");
}

#[test]
fn cross_filtering_drops_only_a_charts_own_dimension() {
    let v = vec![
        canon("/lib/a1.jpg", "RF 50", 5),
        canon("/lib/a2.jpg", "RF 50", 1),
        canon("/lib/a3.jpg", "RF 85", 4),
        Parsed { maker: "Sony".into(), model: "A7".into(), lens: "FE 35".into(), rating: 2, ..photo("/lib/s1.jpg") },
    ];
    let filter = StatsFilter { lenses: Some(vec!["RF 50".into()]), ..f() };
    let s = run(v.clone(), StatsScope::Library, filter);
    // overview and matches use the full filter
    let o = s.overview.unwrap();
    assert_eq!((o.matches.n, o.matches.top), (2, 1));
    // the lens list ignores the lens selection: the alternatives are all there
    let names: Vec<_> = s.lenses.as_ref().unwrap().lenses.iter().map(|l| (l.name.as_str(), l.n)).collect();
    assert_eq!(names, [("RF 50", 2), ("FE 35", 1), ("RF 85", 1)].map(|(a, b)| (a, b)).to_vec().into_iter().fold(
        names.clone(),
        |_, _| names.clone()
    ));
    assert_eq!(s.lenses.as_ref().unwrap().lenses.len(), 3);
    // cameras respect the lens selection
    let c = s.cameras.unwrap();
    assert_eq!((c.known, c.bodies.len(), c.bodies[0].n), (2, 1, 2));
    // a body selection leaves the camera list whole but narrows the lenses
    let filter = StatsFilter { bodies: Some(vec!["Sony|A7".into()]), ..f() };
    let s = run(v.clone(), StatsScope::Library, filter);
    assert_eq!(s.cameras.unwrap().bodies.len(), 2);
    assert_eq!(s.lenses.unwrap().lenses.len(), 1);
    // makers and bodies are one pair: a maker selection also keeps the body list whole
    let filter = StatsFilter { makers: Some(vec!["Sony".into()]), ..f() };
    let c = run(v, StatsScope::Library, filter).cameras.unwrap();
    assert_eq!((c.makers.len(), c.bodies.len()), (2, 2));
}

#[test]
fn a_photo_missing_a_selected_field_never_matches() {
    let v = vec![
        canon("/lib/a.jpg", "RF 50", 5),
        photo("/lib/nodata.jpg"), // no maker, lens, time, exposure, position
    ];
    for filter in [
        StatsFilter { lenses: Some(vec!["RF 50".into()]), ..f() },
        StatsFilter { makers: Some(vec!["Canon".into()]), ..f() },
        StatsFilter { years: Some(vec![2024]), ..f() },
        StatsFilter { hours: Some(vec![12]), ..f() },
        StatsFilter { place: Some(vec!["0".into()]), ..f() },
        StatsFilter { iso_bins: Some(vec![IsoBinId::I100]), ..f() },
    ] {
        let n = run(v.clone(), StatsScope::Library, filter).overview.unwrap().photos;
        assert!(n <= 1);
    }
    // and an unset filter keeps both
    assert_eq!(run(v.clone(), StatsScope::Library, f()).overview.unwrap().photos, 2);
    // unknown buckets stay out of the histograms instead of landing in a zero bucket
    let s = run(v, StatsScope::Library, f());
    let e = s.exposure.unwrap();
    assert_eq!((e.known.aperture, e.known.shutter, e.known.iso), (0, 0, 0));
    assert!(e.aperture.iter().all(|a| a.n == 0));
    assert_eq!(s.time.unwrap().known, 0);
    assert_eq!(s.cameras.unwrap().known, 1);
    assert_eq!(s.places.unwrap().known, 0);
}

#[test]
fn top_images_exclude_unrated_and_rejected_and_sort_by_rating_then_recency() {
    let mut v = Vec::new();
    for (name, rating, rejected, when) in [
        ("five-old", 5, false, "2020:01:01 10:00:00"),
        ("five-new", 5, false, "2024:01:01 10:00:00"),
        ("four-new", 4, false, "2025:01:01 10:00:00"),
        ("three", 3, false, "2025:01:01 10:00:00"),
        ("zero", 0, false, "2025:01:01 10:00:00"),
        ("rejected", 5, true, "2025:01:01 10:00:00"),
        ("five-undated", 5, false, ""),
    ] {
        v.push(Parsed { rating, rejected, taken: at(when), ..photo(&format!("/lib/{name}.jpg")) });
    }
    let s = run(v.clone(), StatsScope::Library, f());
    let top = s.top.unwrap();
    let names: Vec<_> = top.items.iter().map(|p| p.path.trim_start_matches("/lib/")).collect();
    assert_eq!(names, ["five-new.jpg", "five-old.jpg", "five-undated.jpg", "four-new.jpg"]);
    assert_eq!(top.total, 4);
    assert_eq!(top.by_rating, [0, 0, 0, 0, 1, 3]);
    // a threshold of 0 still needs one star and no rejection
    let all = StatsFilter { min_rating: 0, ..f() };
    assert_eq!(run(v.clone(), StatsScope::Library, all).top.unwrap().total, 5);
    // 5-star threshold
    let t5 = StatsFilter { min_rating: 5, ..f() };
    assert_eq!(run(v.clone(), StatsScope::Library, t5).top.unwrap().total, 3);
    // onlyTop restricts every aggregate, matches keep both counts
    let only = StatsFilter { only_top: true, ..f() };
    let o = run(v, StatsScope::Library, only).overview.unwrap();
    assert_eq!((o.matches.n, o.matches.top), (4, 4));
}

#[test]
fn top_pages_continue_where_the_last_ended() {
    let v: Vec<Parsed> = (0..60)
        .map(|i| Parsed { rating: 4 + (i % 2) as u8, taken: at(&format!("2024:01:{:02} 10:00:00", i % 28 + 1)), ..photo(&format!("/lib/p{i:02}.jpg")) })
        .collect();
    let idx = Index::build(v, &[]);
    let scoped = resolve_scope(&idx, &StatsScope::Library, &lib()).unwrap();
    let q = Query::new(&idx, &scoped, &f());
    let (total, first) = top_rows(&q, 0, TOP_PAGE);
    let (_, second) = top_rows(&q, TOP_PAGE, TOP_PAGE);
    let (_, third) = top_rows(&q, 2 * TOP_PAGE, TOP_PAGE);
    assert_eq!((total, first.len(), second.len(), third.len()), (60, 24, 24, 12));
    let mut all: Vec<u32> = [first, second, third].concat();
    let ordered = all.clone();
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), 60);
    // best first across the pages
    let rating = |i: &u32| idx.rows[*i as usize].rating;
    assert!(ordered.windows(2).all(|w| rating(&w[0]) >= rating(&w[1])));
    assert_eq!(ordered.iter().take(30).filter(|i| rating(i) == 5).count(), 30);
}

#[test]
fn scopes_limit_by_folder_and_date_and_hide_what_the_library_hides() {
    let mk = |path: &str, when: &str| Parsed { taken: at(when), ..photo(path) };
    let v = vec![
        mk("/lib/2024/a.jpg", "2024:03:01 10:00:00"),
        mk("/lib/2024/sub/b.jpg", "2024:05:01 10:00:00"),
        mk("/lib/2025/c.jpg", "2025:03:01 10:00:00"),
        mk("/lib/hidden/d.jpg", "2025:03:02 10:00:00"),
        mk("/elsewhere/e.jpg", "2025:03:03 10:00:00"),
        photo("/lib/2024/undated.jpg"),
    ];
    let n = |scope| run(v.clone(), scope, f()).overview.unwrap().photos;
    assert_eq!(n(StatsScope::Library), 4);
    assert_eq!(n(StatsScope::Folder { folder: "/lib/2024".into(), include_subfolders: false }), 2);
    assert_eq!(n(StatsScope::Folder { folder: "/lib/2024/".into(), include_subfolders: true }), 3);
    // a prefix is not a folder: /lib/202 is not /lib/2024
    assert_eq!(n(StatsScope::Folder { folder: "/lib/202".into(), include_subfolders: true }), 0);
    // a hidden folder stays hidden even when asked for by name
    assert_eq!(n(StatsScope::Folder { folder: "/lib/hidden".into(), include_subfolders: true }), 0);
    // dates are inclusive and drop undated photos
    assert_eq!(n(StatsScope::DateRange { from: "2024-03-01".into(), to: "2024-05-01".into() }), 2);
    assert_eq!(n(StatsScope::DateRange { from: "2025-03-01".into(), to: "2024-03-01".into() }), 3);
    assert!(resolve_scope(&Index::build(vec![], &[]), &StatsScope::DateRange { from: "nope".into(), to: "2024-01-01".into() }, &lib()).is_err());
}

#[test]
fn time_buckets_use_local_hours_and_monday_first_weekdays() {
    // 2024-01-01 was a Monday
    let v = vec![
        Parsed { taken: at("2024:01:01 00:00:00"), ..photo("/lib/a.jpg") },
        Parsed { taken: at("2024:01:07 23:59:59"), ..photo("/lib/b.jpg") },
        Parsed { taken: at("2023:12:31 12:00:00"), ..photo("/lib/c.jpg") },
    ];
    let t = run(v.clone(), StatsScope::Library, f()).time.unwrap();
    assert_eq!((t.hours[0].n, t.hours[23].n, t.hours[12].n), (1, 1, 1));
    let w = t.weekdays.unwrap();
    assert_eq!((w[0].n, w[6].n), (1, 2)); // Mon; Sun twice (the 7th and Dec 31)
    assert_eq!((t.months[0].n, t.months[11].n), (2, 1));
    assert_eq!(t.years.iter().map(|y| (y.year, y.n)).collect::<Vec<_>>(), [(2023, 1), (2024, 2)]);
    assert_eq!(t.known, 3);
    // years ignore their own selection, hours respect it
    let filter = StatsFilter { years: Some(vec![2024]), ..f() };
    let t = run(v, StatsScope::Library, filter).time.unwrap();
    assert_eq!(t.years.len(), 2);
    assert_eq!(t.hours.iter().map(|h| h.n).sum::<u32>(), 2);
    assert_eq!(t.months[11].n, 0);
}

#[test]
fn days_and_dates_roundtrip_including_leap_days() {
    for (y, m, d) in [(1970, 1, 1), (2000, 2, 29), (2024, 2, 29), (2100, 3, 1), (1999, 12, 31)] {
        assert_eq!(civil_from_days(days_from_civil(y, m, d)), (y, m, d));
    }
    assert_eq!(fmt_iso(at("2024:02:29 13:05:09").unwrap()), "2024-02-29T13:05:09");
    assert_eq!(fmt_date(at("2024:02:29 13:05:09").unwrap()), "2024-02-29");
    assert!(parse_local("2100:02:29 10:00:00").is_none()); // 2100 is not a leap year
}

#[test]
fn quadkeys_roundtrip_and_nest() {
    let (x, y) = tile_xy(47.6062, -122.3321, 18);
    let key = quadkey(18, x, y);
    assert_eq!(key.len(), 18);
    assert_eq!(parse_quadkey(&key), Some((18, x, y)));
    // a parent's key is a prefix of its children's
    let (px, py) = tile_xy(47.6062, -122.3321, 10);
    assert!(key.starts_with(&quadkey(10, px, py)));
    // Seattle is tile x 2, y 5 at level 4: digits 0, 2, 1, 2
    assert_eq!(quadkey(4, tile_xy(47.6, -122.3, 4).0, tile_xy(47.6, -122.3, 4).1), "0212");
    assert_eq!(quadkey(0, 0, 0), "");
    assert_eq!(parse_quadkey("0230x"), None);
    assert_eq!(parse_quadkey("4"), None);
    assert_eq!(parse_quadkey(&"0".repeat(30)), None);
    // poles and the antimeridian stay inside the grid
    assert_eq!(tile_xy(90.0, 180.0, 22).0, (1 << 22) - 1);
    assert_eq!(tile_xy(-90.0, -180.0, 22), (0, (1 << 22) - 1));
    // tile centres map back into their tile
    let (la, lo) = tile_center(18, x, y);
    assert_eq!(tile_xy(la, lo, 18), (x, y));
}

fn at_pos(path: &str, lat: f64, lng: f64) -> Parsed {
    Parsed { gps: Some((lat, lng)), rating: 4, ..photo(path) }
}

#[test]
fn place_cells_group_by_tile_and_keep_a_solo_pin() {
    let v = vec![
        at_pos("/lib/a.jpg", 47.6062, -122.3321),
        at_pos("/lib/b.jpg", 47.6063, -122.3322),
        at_pos("/lib/c.jpg", 48.8566, 2.3522),
    ];
    let view = PlacesView { zoom: 4, bbox: None, heat: Some(true) };
    let idx = Index::build(v, &[]);
    let p = run_on(&idx, &StatsScope::Library, &f(), Some(view)).places.unwrap();
    assert_eq!((p.known, p.cells.len(), p.top.len()), (3, 2, 2));
    assert_eq!(p.cells[0].n, 2);
    assert!(p.cells[0].solo.is_none() && p.cells[1].solo.is_some());
    assert_eq!(p.cells[0].key.len(), 7);
    assert_eq!(p.heat.unwrap().len(), 2);
    let b = p.bbox.unwrap();
    assert!(b.0 < -122.0 && b.2 > 2.0 && b.1 < 48.0 && b.3 > 48.0);
    // both Seattle photos are one place at the finest level used for the count? 0.1 m apart: yes
    assert_eq!(p.total, 2);
    // a viewport around Paris leaves Seattle out of the cells but not out of `known`
    let view = PlacesView { zoom: 4, bbox: Some((0.0, 40.0, 5.0, 50.0)), heat: None };
    let p = run_on(&idx, &StatsScope::Library, &f(), Some(view)).places.unwrap();
    assert_eq!((p.known, p.cells.len()), (3, 1));
    assert!(p.heat.is_none());
}

#[test]
fn place_cells_are_capped_by_merging_into_parent_tiles() {
    // 40 x 40 positions a few km apart: far more than 1,500 cells at zoom + 3
    let mut v = Vec::new();
    for i in 0..60 {
        for j in 0..60 {
            v.push(at_pos(&format!("/lib/{i}-{j}.jpg"), 10.0 + f64::from(i) * 0.05, 20.0 + f64::from(j) * 0.05));
        }
    }
    let idx = Index::build(v, &[]);
    let view = PlacesView { zoom: 12, bbox: None, heat: None };
    let p = run_on(&idx, &StatsScope::Library, &f(), Some(view)).places.unwrap();
    assert_eq!(p.known, 3600);
    assert!(p.cells.len() <= PLACE_CAP, "{}", p.cells.len());
    assert!(p.cells.len() > 10);
    // merging loses nobody
    assert_eq!(p.cells.iter().map(|c| c.n).sum::<u32>(), 3600);
    // all of one level, shorter than the 15 asked for
    let l = p.cells[0].key.len();
    assert!(l < 15 && p.cells.iter().all(|c| c.key.len() == l));
    // a merged cell's centre is where its photos are
    assert!(p.cells.iter().all(|c| (9.9..=13.1).contains(&c.lat) && (19.9..=23.1).contains(&c.lng)));
    // the quick-jump list is the 12 biggest, before merging
    assert_eq!(p.top.len(), 12);
}

#[test]
fn a_place_filter_matches_by_key_prefix_at_any_level() {
    let v = vec![
        at_pos("/lib/seattle1.jpg", 47.6062, -122.3321),
        at_pos("/lib/seattle2.jpg", 47.65, -122.30),
        at_pos("/lib/paris.jpg", 48.8566, 2.3522),
        photo("/lib/nogps.jpg"),
    ];
    let idx = Index::build(v, &[]);
    let (x, y) = tile_xy(47.6062, -122.3321, 22);
    let seattle_l4 = quadkey(4, x >> 18, y >> 18);
    let n = |keys: Vec<String>| {
        let filter = StatsFilter { place: Some(keys), ..f() };
        run_on(&idx, &StatsScope::Library, &filter, None).overview.unwrap().photos
    };
    assert_eq!(n(vec![seattle_l4.clone()]), 2);
    assert_eq!(n(vec![quadkey(22, x, y)]), 1);
    assert_eq!(n(vec![seattle_l4, quadkey(2, tile_xy(48.85, 2.35, 2).0, tile_xy(48.85, 2.35, 2).1)]), 3);
    // the root key (empty) is every photo with a position; a junk key matches nothing
    assert_eq!(n(vec!["".into()]), 3);
    assert_eq!(n(vec!["9".into()]), 0);
    // the map itself ignores the place selection
    let filter = StatsFilter { place: Some(vec![quadkey(22, x, y)]), ..f() };
    let p = run_on(&idx, &StatsScope::Library, &filter, None).places.unwrap();
    assert_eq!(p.known, 3);
}

#[test]
fn place_detail_summarises_a_cell() {
    let mut a = at_pos("/lib/a.jpg", 47.6062, -122.3321);
    a.maker = "Canon".into();
    a.model = "R5".into();
    a.lens = "RF 50".into();
    a.focal = 50.0;
    a.taken = at("2024:06:01 10:00:00");
    a.rating = 5;
    let mut b = a.clone();
    b.path = "/lib/b.jpg".into();
    b.taken = at("2024:08:01 10:00:00");
    b.rating = 3;
    let far = at_pos("/lib/far.jpg", 10.0, 10.0);
    let idx = Index::build(vec![a, b, far], &[]);
    let scoped = resolve_scope(&idx, &StatsScope::Library, &lib()).unwrap();
    let q = Query::new(&idx, &scoped, &f());
    let (x, y) = tile_xy(47.6062, -122.3321, 22);
    let id = |p: &str| p.to_string();
    let d = place_detail(&q, &Photos { index: &idx, id: &id }, &quadkey(6, x >> 16, y >> 16)).unwrap();
    assert_eq!((d.n, d.top), (2, 1));
    assert_eq!((d.first_shot.as_deref(), d.last_shot.as_deref()), (Some("2024-06-01"), Some("2024-08-01")));
    assert_eq!(d.top_body.unwrap().key, "Canon|R5");
    assert_eq!(d.top_lens.unwrap().name, "RF 50");
    assert_eq!((d.modal_focal_mm, d.months[5], d.months[7]), (Some(50), 1, 1));
    assert_eq!(d.photos[0].path, "/lib/a.jpg"); // best rated first
    assert!(d.radius_km < 1.0);
    assert!(place_detail(&q, &Photos { index: &idx, id: &id }, "9").is_err());
}

#[test]
fn lenses_report_focal_ranges_primes_and_zoom_usage() {
    let mut v = Vec::new();
    for (i, mm) in [24.0, 24.0, 24.0, 35.0, 70.0].into_iter().enumerate() {
        v.push(Parsed { lens: "Zoom 24-70".into(), focal: mm, rating: 4, ..photo(&format!("/lib/z{i}.jpg")) });
    }
    for i in 0..3 {
        v.push(Parsed { lens: "Prime 50".into(), focal: 50.0, ..photo(&format!("/lib/p{i}.jpg")) });
    }
    let l = run(v, StatsScope::Library, f()).lenses.unwrap();
    let zoom = &l.lenses[0];
    assert_eq!(zoom.name, "Zoom 24-70");
    let r = zoom.focal_range.as_ref().unwrap();
    assert_eq!((r.min_mm, r.max_mm, r.modal_mm), (24.0, 70.0, 24.0));
    assert!((r.modal_share - 0.6).abs() < 1e-9);
    assert!(!zoom.is_prime && l.lenses[1].is_prime);
    let usage = &l.zoom_usage.unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!((usage[0].bins.len(), usage[0].bins[0], usage[0].bins[15]), (16, 3, 1));
    assert_eq!(usage[0].bins.iter().sum::<u32>(), 5);
    // the histogram's bins: 1 mm to 100, 5 mm to 300, 25 mm beyond
    assert_eq!([focal_bin(99), focal_bin(100), focal_bin(104), focal_bin(105)], [99, 100, 100, 105]);
    assert_eq!([focal_bin(299), focal_bin(300), focal_bin(324), focal_bin(325)], [295, 300, 300, 325]);
    assert_eq!(l.focal.modal_mm, Some(24));
    assert_eq!(l.focal.bins.iter().map(|b| b.1).sum::<u32>(), 8);
}

#[test]
fn cameras_count_bodies_serials_and_eras() {
    let mut v = Vec::new();
    for (i, (serial, when)) in [("A", "2022:01:01 10:00:00"), ("A", "2023:01:01 10:00:00"), ("B", "2023:06:01 10:00:00")].into_iter().enumerate() {
        v.push(Parsed {
            maker: "Canon".into(),
            model: "R5".into(),
            serial: serial.into(),
            taken: at(when),
            rating: 4,
            ..photo(&format!("/lib/c{i}.jpg"))
        });
    }
    v.push(Parsed { maker: "Sony".into(), model: "A7".into(), taken: at("2023:02:02 10:00:00"), ..photo("/lib/s.jpg") });
    let c = run(v, StatsScope::Library, f()).cameras.unwrap();
    assert_eq!(c.makers[0].bodies, 1);
    let r5 = &c.bodies[0];
    assert_eq!((r5.n, r5.serials, r5.keeper_rate.top), (3, 2, 3));
    assert_eq!((r5.first_shot.as_deref(), r5.last_shot.as_deref()), (Some("2022-01-01"), Some("2023-06-01")));
    assert_eq!(c.years.iter().map(|y| (y.year, y.n, y.cumulative)).collect::<Vec<_>>(), [(2022, 1, 1), (2023, 3, 4)]);
    let tl = c.timeline.unwrap();
    assert_eq!(tl[0].per_year, [1, 2]);
}

#[test]
fn keeper_insights_need_two_comparable_gears() {
    let mut v = Vec::new();
    for i in 0..40 {
        v.push(Parsed { maker: "Canon".into(), model: "R5".into(), rating: if i < 30 { 5 } else { 1 }, ..photo(&format!("/lib/a{i}.jpg")) });
        v.push(Parsed { maker: "Sony".into(), model: "A7".into(), rating: if i < 10 { 5 } else { 1 }, ..photo(&format!("/lib/b{i}.jpg")) });
    }
    let o = run(v, StatsScope::Library, f()).overview.unwrap();
    let k = o.insights.iter().find(|i| i.id == InsightId::KeeperBody).unwrap();
    assert!(matches!(k.params.get("body"), Some(ParamValue::Text(t)) if t == "Canon R5"));
    assert!(matches!(k.params.get("rate"), Some(ParamValue::Number(r)) if *r == 75.0));
    assert_eq!(k.apply.as_ref().unwrap().bodies.as_deref(), Some(&["Canon|R5".to_string()][..]));
    assert!(o.insights.len() <= 6);
}

#[test]
fn medians_are_real_values_and_the_lower_middle_for_even_counts() {
    let mut v = [100.0, 400.0, 200.0, 800.0].map(|iso| Parsed { iso, ..photo(&format!("/lib/{iso}.jpg")) }).to_vec();
    let o = run(v.clone(), StatsScope::Library, f()).overview.unwrap();
    assert_eq!(o.median_setup.iso, Some(200.0));
    v.pop();
    let o = run(v, StatsScope::Library, f()).overview.unwrap();
    assert_eq!(o.median_setup.iso, Some(200.0));
}

#[test]
fn aspect_filter_treats_portrait_as_a_flag() {
    let v = vec![
        Parsed { width: 6000, height: 4000, ..photo("/lib/l.jpg") },
        Parsed { width: 4000, height: 6000, ..photo("/lib/p.jpg") },
        Parsed { width: 4000, height: 3000, ..photo("/lib/f.jpg") },
        photo("/lib/unknown.jpg"),
    ];
    let n = |a: AspectClassId| {
        run(v.clone(), StatsScope::Library, StatsFilter { aspect: Some(vec![a]), ..f() }).overview.unwrap().photos
    };
    assert_eq!((n(AspectClassId::R3x2), n(AspectClassId::Portrait), n(AspectClassId::R4x3), n(AspectClassId::Pano)), (2, 1, 1, 0));
    let o = run(v, StatsScope::Library, f()).overview.unwrap();
    assert_eq!(o.aspect.iter().find(|a| a.id == AspectClassId::Portrait).unwrap().n, 1);
}

#[test]
fn file_types_labels_and_edits_filter() {
    let v = vec![
        Parsed { labels: 1, edited: true, ..photo("/lib/a.ARW") },
        Parsed { labels: 3, ..photo("/lib/b.arw") },
        Parsed { ..photo("/lib/c.jpg") },
    ];
    let o = run(v.clone(), StatsScope::Library, f()).overview.unwrap();
    assert_eq!((o.file_types[0].kind.as_str(), o.file_types[0].n, o.edited), ("ARW", 2, 1));
    assert_eq!((o.labels.red, o.labels.yellow), (2, 1));
    let n = |filter: StatsFilter| run(v.clone(), StatsScope::Library, filter).overview.unwrap().photos;
    assert_eq!(n(StatsFilter { file_types: Some(vec!["arw".into()]), ..f() }), 2);
    assert_eq!(n(StatsFilter { labels: Some(vec![ColorLabel::Yellow, ColorLabel::Blue]), ..f() }), 1);
    assert_eq!(n(StatsFilter { edited: Some(true), ..f() }), 1);
    assert_eq!(n(StatsFilter { edited: Some(false), ..f() }), 2);
}

#[test]
fn coverage_counts_unindexed_photos_in_scope() {
    let idx = Index::build(vec![photo("/lib/a/x.jpg")], &["/lib/a/y.jpg".into(), "/lib/b/z.jpg".into(), "/other/w.jpg".into()]);
    let all = resolve_scope(&idx, &StatsScope::Library, &lib()).unwrap();
    let (indexed, total, per) = scope_counts(&idx, &all);
    assert_eq!((indexed, total), (1, 3));
    assert_eq!(per.iter().map(|p| p.1).sum::<u32>(), 2);
    let folder = resolve_scope(&idx, &StatsScope::Folder { folder: "/lib/a".into(), include_subfolders: false }, &lib()).unwrap();
    assert_eq!(scope_counts(&idx, &folder).1, 2);
    assert!(folder_under(&idx, per[0].0, "/lib"));
}

#[test]
fn grid_rows_are_best_first_and_report_a_cut() {
    let v: Vec<Parsed> = (0..10).map(|i| Parsed { rating: (i % 6) as u8, ..photo(&format!("/lib/{i}.jpg")) }).collect();
    let idx = Index::build(v, &[]);
    let scoped = resolve_scope(&idx, &StatsScope::Library, &lib()).unwrap();
    let q = Query::new(&idx, &scoped, &f());
    let (rows, complete) = grid_rows(&q, 4);
    assert!(!complete);
    assert_eq!(rows.len(), 4);
    assert_eq!(idx.rows[rows[0] as usize].rating, 5);
    assert!(grid_rows(&q, 10).1);
}

#[test]
fn defs_match_the_enums() {
    let d = defs();
    assert_eq!((d.focal.len(), d.aperture.len(), d.shutter.len()), (6, 5, 6));
    assert_eq!(d.focal[0].id, FocalBucketId::Uw);
    assert_eq!(d.shutter[5].to, None);
}

// ---------------------------------------------------------------- scale

fn synthetic(n: usize) -> Vec<Parsed> {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let lenses = ["RF24-105mm F4 L", "RF70-200mm F2.8", "FE 24-70mm F2.8 GM", "FE 85mm F1.8", "XF16-55mm", "iPhone back camera", "Z 50mm f/1.8", "RF50mm F1.8"];
    let bodies = [("Canon", "EOS R5"), ("Sony", "ILCE-7M4"), ("Fujifilm", "X-T5"), ("Apple", "iPhone 15 Pro"), ("Nikon", "Z 6_2")];
    (0..n)
        .map(|i| {
            let r = next();
            let (maker, model) = bodies[(r % 5) as usize];
            let year = 2008 + (r >> 8) % 18;
            let t = at(&format!("{year}:{:02}:{:02} {:02}:{:02}:00", 1 + (r >> 16) % 12, 1 + (r >> 20) % 28, (r >> 26) % 24, (r >> 32) % 60));
            Parsed {
                path: format!("/lib/{year}/{:03}/IMG_{i:07}.jpg", (r >> 40) % 200),
                maker: maker.into(),
                model: model.into(),
                lens: lenses[((r >> 3) % 8) as usize].into(),
                serial: format!("{}", (r >> 12) % 7),
                rating: ((r >> 44) % 6) as u8,
                rejected: (r >> 50) % 40 == 0,
                labels: if (r >> 52) % 9 == 0 { 1 << ((r >> 56) % 5) } else { 0 },
                edited: (r >> 58) % 5 == 0,
                width: 6000,
                height: if (r >> 14) % 4 == 0 { 8000 } else { 4000 },
                focal: [16.0, 24.0, 35.0, 50.0, 85.0, 105.0, 200.0, 400.0][((r >> 9) % 8) as usize],
                aperture: [1.4, 1.8, 2.8, 4.0, 5.6, 8.0, 11.0, 16.0][((r >> 6) % 8) as usize],
                shutter: 1.0 / [8000.0, 2000.0, 500.0, 125.0, 30.0, 4.0][((r >> 18) % 6) as usize],
                iso: [100.0, 200.0, 400.0, 800.0, 3200.0, 12800.0][((r >> 22) % 6) as usize],
                taken: t,
                gps: if r % 10 < 7 {
                    Some((20.0 + (r >> 4) as f64 % 40.0 + (r >> 30) as f64 % 1000.0 / 1000.0, -120.0 + (r >> 5) as f64 % 120.0 + (r >> 34) as f64 % 1000.0 / 1000.0))
                } else {
                    None
                },
                ..Default::default()
            }
        })
        .collect()
}

#[test]
fn a_synthetic_library_adds_up() {
    let n = 5000;
    let idx = Index::build(synthetic(n), &[]);
    let s = run_on(&idx, &StatsScope::Library, &f(), None);
    let o = s.overview.unwrap();
    assert_eq!(o.photos as usize, n);
    assert_eq!(o.rating_dist.iter().sum::<u32>() + o.rejected, n as u32);
    let c = s.cameras.unwrap();
    assert_eq!(c.bodies.iter().map(|b| b.n).sum::<u32>(), n as u32);
    assert_eq!(s.exposure.unwrap().iso.iter().map(|i| i.n).sum::<u32>(), n as u32);
    let t = s.time.unwrap();
    assert_eq!(t.hours.iter().map(|h| h.n).sum::<u32>(), n as u32);
    assert_eq!(s.lenses.unwrap().lenses.iter().map(|l| l.n).sum::<u32>(), n as u32);
    assert_eq!(s.places.unwrap().cells.iter().map(|c| c.n).sum::<u32>(), s_known(&idx));
}
fn s_known(idx: &Index) -> u32 {
    idx.rows.iter().filter(|r| r.has_gps()).count() as u32
}

/// `cargo test --release --locked stats::tests::benchmark -- --ignored --nocapture`
#[test]
#[ignore]
fn benchmark_300k() {
    let n = 300_000;
    let t = std::time::Instant::now();
    let parsed = synthetic(n);
    println!("generate   {:>7.0} ms", t.elapsed().as_secs_f64() * 1000.0);
    let t = std::time::Instant::now();
    let idx = Index::build(parsed, &[]);
    println!("index      {:>7.0} ms  ({} bodies, {} lenses, {} folders)", t.elapsed().as_secs_f64() * 1000.0, idx.bodies.len(), idx.lenses.len(), idx.folders.len());
    let id = |p: &str| p.to_string();
    let view = Some(PlacesView { zoom: 5, bbox: None, heat: Some(true) });
    let cases: [(&str, StatsFilter); 3] = [
        ("no filter", f()),
        ("lens+year", StatsFilter { lenses: Some(vec!["FE 85mm F1.8".into()]), years: Some(vec![2019, 2020]), ..f() }),
        ("only top", StatsFilter { only_top: true, ..f() }),
    ];
    for (name, filter) in cases {
        let best = (0..5)
            .map(|_| {
                let t = std::time::Instant::now();
                let scoped = resolve_scope(&idx, &StatsScope::Library, &lib()).unwrap();
                let q = Query::new(&idx, &scoped, &filter);
                let s = snapshot(&q, &Photos { index: &idx, id: &id }, &Wanted { sections: &ALL, places: view.clone() });
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                let size = {
                    let o = serde_json::to_string(&s.overview).unwrap().len()
                        + serde_json::to_string(&s.cameras).unwrap().len()
                        + serde_json::to_string(&s.lenses).unwrap().len()
                        + serde_json::to_string(&s.exposure).unwrap().len()
                        + serde_json::to_string(&s.time).unwrap().len()
                        + serde_json::to_string(&s.places).unwrap().len()
                        + serde_json::to_string(&s.top).unwrap().len();
                    o
                };
                (ms, size)
            })
            .fold((f64::MAX, 0), |a, b| (a.0.min(b.0), b.1));
        println!("snapshot [{name:<10}] best of 5: {:>6.0} ms, payload {} KB", best.0, best.1 / 1024);
        let scoped = resolve_scope(&idx, &StatsScope::Library, &lib()).unwrap();
        let q = Query::new(&idx, &scoped, &filter);
        let s = snapshot(&q, &Photos { index: &idx, id: &id }, &Wanted { sections: &ALL, places: view.clone() });
        let kb = |v: String| v.len() / 1024;
        println!(
            "   KB: overview {} cameras {} lenses {} exposure {} time {} places {} top {}",
            kb(serde_json::to_string(&s.overview).unwrap()), kb(serde_json::to_string(&s.cameras).unwrap()),
            kb(serde_json::to_string(&s.lenses).unwrap()), kb(serde_json::to_string(&s.exposure).unwrap()),
            kb(serde_json::to_string(&s.time).unwrap()), kb(serde_json::to_string(&s.places).unwrap()),
            kb(serde_json::to_string(&s.top).unwrap())
        );
    }
}

// ---------------------------------------------------------------- the cache

use crate::search_cache::{BatchItem, SearchCache};

fn meta_json(rating: i64, labels: u32, w: u32, h: u32, loc: Option<(f64, f64)>, exif: serde_json::Value) -> String {
    serde_json::json!({
        "rating": rating.max(0), "rejected": rating < 0, "labels": labels, "edited": false,
        "width": w, "height": h,
        "location": loc.map(|(a, b)| serde_json::json!({"latitude": a, "longitude": b})),
        "exif": exif,
    })
    .to_string()
}

#[test]
fn the_cache_reads_in_pages_and_counts_unanalysed_photos() {
    let dir = tempfile::tempdir().unwrap();
    let cache = SearchCache::open(&dir.path().join("c.sqlite")).unwrap();
    let total = 5000; // more than two pages of 2,048
    let metas: Vec<(String, Option<String>)> = (0..total)
        .map(|i| {
            let path = format!("/lib/f{}/img{i}.jpg", i % 7);
            // every 10th photo is still waiting for its metadata
            let m = (i % 10 != 0).then(|| meta_json(i as i64 % 6, 0, 600, 400, None, serde_json::json!({"maker":"SONY","model":"A7","datetime":"2024:05:05 10:00:00"})));
            (path, m)
        })
        .collect();
    let items: Vec<BatchItem> = metas
        .iter()
        .map(|(p, m)| BatchItem {
            path: p,
            copy_key: "",
            original: "o",
            edited: "e",
            capture_time_ns: None,
            capture_time_raw: None,
            timezone_offset_seconds: None,
            metadata: m.as_deref(),
        })
        .collect();
    cache.upsert_metadata_batch(&items).unwrap();
    // a virtual copy is not another photo
    cache
        .upsert_metadata_batch(&[BatchItem { path: "/lib/f0/img1.jpg", copy_key: "version:1", original: "o", edited: "e2", capture_time_ns: None, capture_time_raw: None, timezone_offset_seconds: None, metadata: Some("{}") }])
        .unwrap();
    let idx = crate::stats_cmds::load(&cache).unwrap();
    assert_eq!(idx.rows.len(), 4500);
    assert_eq!(idx.unindexed.iter().map(|u| u.1).sum::<u32>(), 500);
    let s = run_on(&idx, &StatsScope::Library, &f(), None);
    let o = s.overview.unwrap();
    assert_eq!((o.photos, o.folders), (4500, 7));
    let cov = scope_counts(&idx, &resolve_scope(&idx, &StatsScope::Library, &lib()).unwrap());
    assert_eq!((cov.0, cov.1), (4500, 5000));
    // the capture columns: an instant with its offset places a photo in local time
    let c2 = SearchCache::open(&dir.path().join("c2.sqlite")).unwrap();
    c2.upsert_metadata_batch(&[BatchItem {
        path: "/lib/tz.jpg",
        copy_key: "",
        original: "o",
        edited: "e",
        capture_time_ns: Some(1_720_078_200_000_000_000),
        capture_time_raw: None,
        timezone_offset_seconds: Some(9 * 3600),
        metadata: Some("{\"exif\":{}}"),
    }])
    .unwrap();
    let idx = crate::stats_cmds::load(&c2).unwrap();
    let t = run_on(&idx, &StatsScope::Library, &f(), None).time.unwrap();
    assert_eq!(t.hours[16].n, 1); // 07:30 UTC in UTC+9
}

fn full_meta(path: &str, rating: u8) -> String {
    serde_json::json!({
        "imgid": 0, "path": path, "filename": std::path::Path::new(path).file_name().unwrap().to_str().unwrap(),
        "rating": rating, "rejected": false, "labels": 0, "edited": false, "width": 600, "height": 400,
        "location": null, "keywords": [], "version": 0, "copy": false,
        "exif": {"maker": "SONY", "model": "A7", "lens": "", "exposure": 0.01, "aperture": 2.8, "iso": 100,
                 "focal_length": 50, "exposure_bias": 0, "focus_distance": 0, "datetime": "2024:05:05 10:00:00"},
    })
    .to_string()
}

#[test]
fn grid_entries_turn_handles_into_library_rows_in_order() {
    use crate::library_ids::Photo;
    let dir = tempfile::tempdir().unwrap();
    let cache = SearchCache::open(&dir.path().join("g.sqlite")).unwrap();
    let paths = ["/lib/a.jpg", "/lib/b.jpg", "/lib/c.jpg"];
    let metas: Vec<String> = paths.iter().enumerate().map(|(i, p)| full_meta(p, i as u8 + 1)).collect();
    let item = |i: usize, meta: Option<&'static str>| BatchItem {
        path: paths[i], copy_key: "", original: "o", edited: "e", capture_time_ns: None,
        capture_time_raw: None, timezone_offset_seconds: None, metadata: meta.or(Some(metas[i].as_str())),
    };
    // c has no metadata yet: it is not analysed, so it has no grid row
    let c = BatchItem { metadata: None, ..item(2, None) };
    cache.upsert_metadata_batch(&[item(0, None), item(1, None), c]).unwrap();
    let handle = |i: i64| 1_i64 << 40 | i;
    let resolve = |id: i64| match id {
        h if h == handle(1) => Some(Photo { path: paths[0].into(), version: 0 }),
        h if h == handle(2) => Some(Photo { path: paths[1].into(), version: 0 }),
        h if h == handle(3) => Some(Photo { path: paths[2].into(), version: 0 }),
        h if h == handle(4) => Some(Photo { path: paths[1].into(), version: 2 }), // a copy: not a statistics handle
        _ => None,
    };
    let ids: Vec<String> = [3, 2, 99, 1, 4].iter().map(|i| handle(*i).to_string()).chain(["junk".to_string()]).collect();
    let rows = crate::stats_cmds::grid_entries(&cache, &ids, &resolve).unwrap();
    // asked order kept; unknown, unparsable, copy and unanalysed handles dropped
    assert_eq!(rows.iter().map(|r| (r.imgid, r.path.as_str(), r.rating)).collect::<Vec<_>>(),
               vec![(handle(2), "/lib/b.jpg", 2), (handle(1), "/lib/a.jpg", 1)]);
    // the 20,000 cap of stats_open_in_grid is the most the grid takes
    let many: Vec<String> = (0..20_001).map(|i| i.to_string()).collect();
    assert!(crate::stats_cmds::grid_entries(&cache, &many, &resolve).is_err());
    assert!(crate::stats_cmds::grid_entries(&cache, &many[..20_000], &resolve).unwrap().is_empty());
}

// ---------------------------------------------------------------- ground truth

const GROUND_TRUTH: &str = "/tmp/claude-1000/-home-ssarangi-dev-immage/edf2a6bc-7fec-4393-8f7c-9e37ed2ca1cb/scratchpad/stats-library";

fn csv_line(line: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted) = (vec![], String::new(), false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

type GtRow = (String, String, Option<String>, Option<i32>);
/// The ground-truth manifest as the indexer would store it: path, metadata JSON, capture clock, offset.
fn gt_rows(manifest: &[Vec<String>], head: &[String], root: &str) -> Vec<GtRow> {
    let col = |row: &[String], name: &str| row[head.iter().position(|h| h == name).unwrap()].clone();
    let mut keep: Vec<GtRow> = vec![];
    for r in manifest {
        let c = |n: &str| col(r, n);
        let num = |n: &str| c(n).parse::<f64>().unwrap_or(0.0);
        let focal = num("focal");
        let labels = match c("label").as_str() {
            "red" => 1,
            "yellow" => 2,
            "green" => 4,
            "blue" => 8,
            "purple" => 16,
            _ => 0,
        };
        let loc = (!c("lat").is_empty()).then(|| (num("lat"), num("lon")));
        let dt = c("dt").replace('-', ":");
        let offset = {
            let o = c("offset");
            (o.len() == 6).then(|| (if o.starts_with('-') { -1 } else { 1 }) * (o[1..3].parse::<i32>().unwrap() * 3600 + o[4..6].parse::<i32>().unwrap() * 60))
        };
        let exif = serde_json::json!({
            "maker": c("make"), "model": c("model"), "lens": c("lens"),
            "exposure": num("shutter_s"), "aperture": num("aperture"), "iso": num("iso"),
            "focal_length": focal, "crop_factor": if focal > 0.0 { num("focal35") / focal } else { 0.0 },
            "datetime": dt,
        });
        let meta = meta_json(c("rating").parse().unwrap(), labels, c("w").parse().unwrap(), c("h").parse().unwrap(), loc, exif);
        keep.push((format!("{root}/{}", c("path")), meta, (!dt.is_empty()).then_some(dt), offset));
    }
    keep
}

#[test]
fn aggregates_equal_the_generated_libraries_ground_truth() {
    use std::collections::BTreeMap;
    let dir = std::path::Path::new(GROUND_TRUTH);
    if !dir.join("manifest.csv").exists() || !dir.join("expected.json").exists() {
        eprintln!("SKIPPED: ground-truth library not found at {GROUND_TRUTH}");
        return;
    }
    let text = std::fs::read_to_string(dir.join("manifest.csv")).unwrap();
    let mut lines = text.lines();
    let head = csv_line(lines.next().unwrap());
    let col = |row: &[String], name: &str| row[head.iter().position(|h| h == name).unwrap()].clone();
    let manifest: Vec<Vec<String>> = lines.filter(|l| !l.is_empty()).map(csv_line).collect();
    let expected: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("expected.json")).unwrap()).unwrap();
    let ag = &expected["aggregates"];
    let root = "/photos-root";

    // a temp cache built straight from the manifest, as the indexer would have stored it
    let tmp = tempfile::tempdir().unwrap();
    let cache = SearchCache::open(&tmp.path().join("gt.sqlite")).unwrap();
    let keep = gt_rows(&manifest, &head, root);
    let items: Vec<BatchItem> = keep
        .iter()
        .map(|(p, m, dt, off)| BatchItem {
            path: p,
            copy_key: "",
            original: "o",
            edited: "e",
            capture_time_ns: None,
            capture_time_raw: dt.as_deref(),
            timezone_offset_seconds: *off,
            metadata: Some(m),
        })
        .collect();
    cache.upsert_metadata_batch(&items).unwrap();
    let idx = crate::stats_cmds::load(&cache).unwrap();
    let library = Library { roots: vec![root.into()], hidden: vec![] };
    let scoped = resolve_scope(&idx, &StatsScope::Library, &library).unwrap();
    let q = Query::new(&idx, &scoped, &f());
    let id = |p: &str| p.to_string();
    let s = snapshot(&q, &Photos { index: &idx, id: &id }, &Wanted { sections: &ALL, places: Some(PlacesView { zoom: 3, bbox: None, heat: None }) });
    let to_map = |v: &serde_json::Value| -> BTreeMap<String, u32> { v.as_object().unwrap().iter().map(|(k, n)| (k.clone(), n.as_u64().unwrap() as u32)).collect() };

    // overview
    let o = s.overview.unwrap();
    assert_eq!(o.photos, expected["photos"].as_u64().unwrap() as u32);
    let per_rating = to_map(&ag["per_rating"]);
    assert_eq!(o.rejected, per_rating["-1"]);
    assert_eq!(o.rating_dist.to_vec(), (0..=5).map(|i| per_rating[&i.to_string()]).collect::<Vec<_>>());
    let per_label = to_map(&ag["per_label"]);
    assert_eq!((o.labels.red, o.labels.yellow, o.labels.green, o.labels.blue, o.labels.purple), (per_label["red"], per_label["yellow"], per_label["green"], per_label["blue"], per_label["purple"]));
    assert_eq!(o.gear.bodies, 6 - 0, "six camera models");
    let top_n = per_rating["4"] + per_rating["5"];
    assert_eq!(o.matches.top, top_n);
    assert_eq!(s.top.as_ref().unwrap().total, top_n);

    // cameras: photos without a Make have no body, so each model is short by exactly those
    let c = s.cameras.unwrap();
    let missing_make = |model: &str| manifest.iter().filter(|r| col(r, "model") == model && col(r, "make").is_empty()).count() as u32;
    for (model, n) in to_map(&ag["per_camera"]) {
        let found = c.bodies.iter().find(|b| b.model == model).map_or(0, |b| b.n);
        assert_eq!(found + missing_make(&model), n, "camera {model}");
    }
    let unknown_make = to_map(&ag["per_make"])["Unknown"];
    assert_eq!(c.known, expected["photos"].as_u64().unwrap() as u32 - unknown_make);
    let mut makers: BTreeMap<String, u32> = BTreeMap::new();
    for (m, n) in to_map(&ag["per_make"]) {
        if m != "Unknown" {
            *makers.entry(norm_maker(&m)).or_default() += n;
        }
    }
    assert_eq!(c.makers.iter().map(|m| (m.maker.clone(), m.n)).collect::<BTreeMap<_, _>>(), makers);

    // lenses
    let l = s.lenses.unwrap();
    let mut lens_expected = to_map(&ag["per_lens"]);
    let unknown_lens = lens_expected.remove("Unknown").unwrap();
    assert_eq!(l.known, expected["photos"].as_u64().unwrap() as u32 - unknown_lens);
    assert_eq!(l.lenses.iter().map(|x| (x.name.clone(), x.n)).collect::<BTreeMap<_, _>>(), lens_expected);

    // time: undated photos only count under "no-datetime"
    let t = s.time.unwrap();
    let per_year = to_map(&ag["per_year"]);
    assert_eq!(t.known, expected["photos"].as_u64().unwrap() as u32 - per_year["no-datetime"]);
    assert_eq!(t.years.iter().map(|y| (y.year.to_string(), y.n)).collect::<BTreeMap<_, _>>(), per_year.iter().filter(|(k, _)| *k != "no-datetime").map(|(k, v)| (k.clone(), *v)).collect());
    let per_hour = to_map(&ag["per_hour"]);
    for (h, c) in t.hours.iter().enumerate() {
        assert_eq!(c.n, per_hour.get(&format!("{h:02}")).copied().unwrap_or(0), "hour {h}");
    }
    let per_month = to_map(&ag["per_year_month"]);
    for m in 1..=12 {
        let want: u32 = per_month.iter().filter(|(k, _)| k[5..].parse::<u32>().unwrap() == m).map(|(_, v)| v).sum();
        assert_eq!(t.months[m as usize - 1].n, want, "month {m}");
    }
    let per_wd = to_map(&ag["per_weekday"]);
    for (i, d) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].iter().enumerate() {
        assert_eq!(t.weekdays.as_ref().unwrap()[i].n, per_wd[*d], "weekday {d}");
    }

    // places
    let p = s.places.unwrap();
    assert_eq!(p.known, ag["gps"]["with_gps"].as_u64().unwrap() as u32);
    assert_eq!(p.cells.iter().map(|c| c.n).sum::<u32>(), p.known);

    // exposure, by the contract's own edges (read from the defs, not from the code's constants)
    let e = s.exposure.unwrap();
    let d = defs();
    let mut ap = [0u32; 5];
    let mut sh = [0u32; 6];
    for r in &manifest {
        let f: f64 = col(r, "aperture").parse().unwrap();
        let g = d.aperture.iter().position(|a| a.to_f.is_none_or(|t| f <= t + 0.005)).unwrap();
        ap[g] += 1;
        let t: f64 = col(r, "shutter_s").parse().unwrap();
        sh[d.shutter.iter().position(|x| x.to.is_none_or(|e| t < e)).unwrap()] += 1;
    }
    assert_eq!(e.aperture.iter().map(|a| a.n).collect::<Vec<_>>(), ap);
    assert_eq!(e.shutter.iter().map(|a| a.n).collect::<Vec<_>>(), sh);
    assert_eq!(e.iso.iter().map(|i| i.n).sum::<u32>(), 400);

    // per-lens top frames: five stars of one lens, newest first, as the ground truth ranks them
    for (lens, v) in ag["per_lens_rating"].as_object().unwrap() {
        if lens == "Unknown" {
            continue;
        }
        let want: Vec<String> = v["top10_five_star"].as_array().unwrap().iter().map(|p| format!("{root}/{}", p.as_str().unwrap())).collect();
        let filter = StatsFilter { min_rating: 5, lenses: Some(vec![lens.clone()]), ..Default::default() };
        let q = Query::new(&idx, &scoped, &filter);
        let (total, rows) = top_rows(&q, 0, 10);
        assert_eq!(total, v["five_star"].as_u64().unwrap() as u32, "{lens}");
        let got: Vec<String> = rows.iter().map(|i| idx.rows[*i as usize].path.to_string()).collect();
        assert_eq!(got, want[..want.len().min(10)], "{lens}");
    }
    eprintln!("ground truth: {} photos matched expected.json", o.photos);
}

/// Reading 300,000 stored rows (about 1 KB of metadata each) from SQLite into an index.
#[test]
#[ignore]
fn benchmark_300k_load() {
    let dir = tempfile::tempdir().unwrap();
    let cache = SearchCache::open(&dir.path().join("b.sqlite")).unwrap();
    let t = std::time::Instant::now();
    for chunk in synthetic(300_000).chunks(5000) {
        let metas: Vec<String> = chunk
            .iter()
            .map(|p| {
                let exif = serde_json::json!({
                    "maker": p.maker, "model": p.model, "lens": p.lens, "body_serial": p.serial,
                    "exposure": p.shutter, "aperture": p.aperture, "iso": p.iso, "focal_length": p.focal,
                    "exposure_bias": -3.4e38, "focus_distance": 0.0, "crop_factor": 0.0, "white_balance": "Auto",
                    "flash": "No flash", "exposure_program": "Aperture priority", "metering_mode": "Matrix",
                    "file_width": 6000, "file_height": 4000, "sensor_width": 6000, "sensor_height": 4000,
                    "datetime": p.taken.map_or(String::new(), |t| fmt_iso(t).replace('-', ":").replace('T', " ")),
                });
                let mut v: serde_json::Value = serde_json::from_str(&meta_json(i64::from(p.rating), 0, 6000, 4000, p.gps, exif)).unwrap();
                v["path"] = p.path.clone().into();
                v["filename"] = "IMG.jpg".into();
                v["keywords"] = serde_json::json!(["travel|city", "street", "night"]);
                v["metadata"] = serde_json::json!({"title": "", "description": "", "creator": "A Photographer", "publisher": "", "rights": "All rights reserved", "notes": ""});
                v.to_string()
            })
            .collect();
        let items: Vec<BatchItem> = chunk
            .iter()
            .zip(&metas)
            .map(|(p, m)| BatchItem { path: &p.path, copy_key: "", original: "o", edited: "e", capture_time_ns: None, capture_time_raw: None, timezone_offset_seconds: None, metadata: Some(m) })
            .collect();
        cache.upsert_metadata_batch(&items).unwrap();
    }
    println!("fill cache {:>7.0} ms", t.elapsed().as_secs_f64() * 1000.0);
    for _ in 0..3 {
        let t = std::time::Instant::now();
        let idx = crate::stats_cmds::load(&cache).unwrap();
        println!("load index {:>7.0} ms ({} rows)", t.elapsed().as_secs_f64() * 1000.0, idx.rows.len());
    }
}

// ---------------------------------------------------------------- harness fixtures

fn fixture_requests(idx: &Index, library: &Library) -> Vec<StatsRequest> {
    let sections = vec![StatsSection::Overview, StatsSection::Cameras, StatsSection::Lenses, StatsSection::Exposure, StatsSection::Time];
    let req = |scope: StatsScope, filter: StatsFilter| StatsRequest { scope, filter, sections: sections.clone(), year_mode: None, etag: None, places: None };
    let lib = StatsScope::Library;
    let base = f();
    let id = |p: &str| p.to_string();
    let scoped = resolve_scope(idx, &lib, library).unwrap();
    let first = snapshot(&Query::new(idx, &scoped, &base), &Photos { index: idx, id: &id }, &Wanted { sections: &ALL, places: None });
    let (cam, lens, ov) = (first.cameras.unwrap(), first.lenses.unwrap(), first.overview.unwrap());
    let mut out = vec![req(lib.clone(), base.clone())];
    for min in [1u8, 2, 3, 5] {
        out.push(req(lib.clone(), StatsFilter { min_rating: min, ..base.clone() }));
    }
    out.push(req(lib.clone(), StatsFilter { only_top: true, ..base.clone() }));
    // the filters the page applies by one click, three biggest values of each chart
    for m in cam.makers.iter().take(3) {
        out.push(req(lib.clone(), StatsFilter { makers: Some(vec![m.maker.clone()]), ..base.clone() }));
    }
    for b in cam.bodies.iter().take(3) {
        out.push(req(lib.clone(), StatsFilter { bodies: Some(vec![b.key.clone()]), ..base.clone() }));
    }
    for l in lens.lenses.iter().take(3) {
        out.push(req(lib.clone(), StatsFilter { lenses: Some(vec![l.name.clone()]), ..base.clone() }));
    }
    for y in ov.per_year.iter().take(4) {
        out.push(req(lib.clone(), StatsFilter { years: Some(vec![y.year]), ..base.clone() }));
    }
    for b in lens.focal_by_bucket.iter().filter(|b| b.n > 0) {
        out.push(req(lib.clone(), StatsFilter { focal_buckets: Some(vec![b.id]), ..base.clone() }));
    }
    for m in [10u8, 6, 7] {
        out.push(req(lib.clone(), StatsFilter { months: Some(vec![m]), ..base.clone() }));
    }
    for h in [18u8, 7, 12] {
        out.push(req(lib.clone(), StatsFilter { hours: Some(vec![h]), ..base.clone() }));
    }
    out.push(req(lib.clone(), StatsFilter { aperture_groups: Some(vec![ApertureGroupId::A3]), ..base.clone() }));
    out.push(req(lib.clone(), StatsFilter { shutter_regimes: Some(vec![ShutterRegimeId::Handheld]), ..base.clone() }));
    out.push(req(lib.clone(), StatsFilter { iso_bins: Some(vec![IsoBinId::I100]), ..base.clone() }));
    out.push(req(lib.clone(), StatsFilter { iso_bins: Some(vec![IsoBinId::I400]), ..base.clone() }));
    out.push(req(lib.clone(), StatsFilter { labels: Some(vec![ColorLabel::Red]), ..base.clone() }));
    // the harness's named states: a combination, a range scope, a selection that matches nothing
    out.push(req(
        lib.clone(),
        StatsFilter { focal_buckets: Some(vec![FocalBucketId::Normal]), months: Some(vec![10]), makers: Some(vec!["Canon".into()]), ..base.clone() },
    ));
    out.push(req(StatsScope::DateRange { from: "2024-01-01".into(), to: "2024-12-31".into() }, base.clone()));
    out.push(req(lib, StatsFilter { makers: Some(vec!["Nobody".into()]), ..base }));
    out
}

/// The Top images section's recorded replies: for every request the page can make (`fixture_requests`), the
/// `top` block alone, with its cursor, and the pages that cursor leads to (`page` entries: the cursor's scope,
/// filter and offset, as `stats_top_page` decodes them, and its reply). The base request goes six pages deep, the others three.
fn top_fixtures(idx: &Index, library: &Library, handle: &dyn Fn(&str) -> String) -> Vec<serde_json::Value> {
    let photos = Photos { index: idx, id: handle };
    let cursor = |scope: &StatsScope, filter: &StatsFilter, offset: usize| serde_json::json!({ "scope": scope, "filter": filter, "offset": offset, "gen": 1 }).to_string();
    let mut out = vec![];
    let mut seen: Vec<String> = vec![];
    for (n, base) in fixture_requests(idx, library).into_iter().enumerate() {
        let request = StatsRequest { sections: vec![StatsSection::Top], ..base };
        let key = serde_json::to_string(&(&request.scope, &request.filter)).unwrap();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let scoped = resolve_scope(idx, &request.scope, library).unwrap();
        let q = Query::new(idx, &scoped, &request.filter);
        let s = snapshot(&q, &photos, &Wanted { sections: &request.sections, places: None });
        let (indexed, total, _) = scope_counts(idx, &scoped);
        let mut top = s.top.unwrap();
        if (top.items.len() as u32) < top.total {
            top.page.next = Some(cursor(&request.scope, &request.filter, top.items.len()));
        }
        let reply = StatsResponse {
            etag: format!("fixture-top-{n}"),
            generated_at: "2026-11-21T00:00:00+00:00".into(),
            coverage: Coverage { indexed, total: Some(total), pending: 0, roots: 1, pending_roots: vec![], paused: false, max_age_sec: 0, state: CoverageState::Ready },
            defs: defs(),
            overview: None,
            cameras: None,
            lenses: None,
            exposure: None,
            time: None,
            places: None,
            top: Some(top),
        };
        out.push(serde_json::json!({ "request": request, "reply": StatsReply::Full(Box::new(reply)) }));
        let depth = if n == 0 { 6 } else { 3 };
        let mut offset = TOP_PAGE;
        for _ in 1..depth {
            let (total, rows) = top_rows(&q, offset, TOP_PAGE);
            if rows.is_empty() {
                break;
            }
            let next = (((offset + rows.len()) as u32) < total).then(|| cursor(&request.scope, &request.filter, offset + rows.len()));
            let page = TopPageReply { items: rows.iter().map(|i| photos.get(*i)).collect(), next };
            out.push(serde_json::json!({ "page": { "scope": request.scope, "filter": request.filter, "offset": offset }, "reply": page }));
            offset += rows.len();
        }
    }
    out
}

/// The Places section's recorded replies: the map's cells at the zooms the harness shows (the whole world,
/// `bbox` absent: the harness cannot know what the viewer pans to, so the page draws what is in view), with and
/// without the filters the page applies, one `stats_place` reply for every big cell, and the three main
/// sections under a place filter. Entries with a `place` key are `stats_place` calls.
fn place_fixtures(idx: &Index, library: &Library, handle: &dyn Fn(&str) -> String) -> Vec<serde_json::Value> {
    let photos = Photos { index: idx, id: handle };
    let lib = StatsScope::Library;
    let base = f();
    let scoped = resolve_scope(idx, &lib, library).unwrap();
    let reply_of = |request: &StatsRequest, n: usize| {
        let scoped = resolve_scope(idx, &request.scope, library).unwrap();
        let q = Query::new(idx, &scoped, &request.filter);
        let s = snapshot(&q, &photos, &Wanted { sections: &request.sections, places: request.places.clone() });
        let (indexed, total, _) = scope_counts(idx, &scoped);
        let reply = StatsResponse {
            etag: format!("fixture-places-{n}"),
            generated_at: "2026-11-21T00:00:00+00:00".into(),
            coverage: Coverage { indexed, total: Some(total), pending: 0, roots: 1, pending_roots: vec![], paused: false, max_age_sec: 0, state: CoverageState::Ready },
            defs: defs(),
            overview: s.overview,
            cameras: s.cameras,
            lenses: s.lenses,
            exposure: s.exposure,
            time: s.time,
            places: s.places,
            top: s.top,
        };
        serde_json::json!({ "request": request, "reply": StatsReply::Full(Box::new(reply)) })
    };
    let map = |filter: StatsFilter, zoom: u8, heat: bool| StatsRequest {
        scope: lib.clone(),
        filter,
        sections: vec![StatsSection::Places],
        year_mode: None,
        etag: None,
        places: Some(PlacesView { zoom, bbox: None, heat: heat.then_some(true) }),
    };
    let mut out = vec![];
    let (top_maker, _) = {
        let q = Query::new(idx, &scoped, &base);
        let s = snapshot(&q, &photos, &Wanted { sections: &[StatsSection::Cameras], places: None });
        (s.cameras.unwrap().makers[0].maker.clone(), 0)
    };
    let filters = [
        base.clone(),
        StatsFilter { only_top: true, ..base.clone() },
        StatsFilter { makers: Some(vec![top_maker]), ..base.clone() },
        StatsFilter { focal_buckets: Some(vec![FocalBucketId::Normal]), ..base.clone() },
        StatsFilter { months: Some(vec![10]), ..base.clone() },
        StatsFilter { makers: Some(vec!["Nobody".into()]), ..base.clone() },
    ];
    for zoom in 1u8..=12 {
        out.push(reply_of(&map(base.clone(), zoom, zoom == 4 || zoom == 6), out.len()));
        out.push(reply_of(&map(base.clone(), zoom, true), out.len()));
    }
    for filter in &filters[1..] {
        for zoom in [2u8, 4, 6] {
            out.push(reply_of(&map(filter.clone(), zoom, false), out.len()));
        }
    }
    // the selection: details of every cell at the low zooms and of the 12 biggest at the others
    let mut keys: Vec<String> = vec![];
    for zoom in 1u8..=12 {
        let q = Query::new(idx, &scoped, &base);
        let s = snapshot(&q, &photos, &Wanted { sections: &[StatsSection::Places], places: Some(PlacesView { zoom, bbox: None, heat: None }) });
        let pl = s.places.unwrap();
        let take = if zoom <= 3 { pl.cells.len() } else { 12 };
        for c in pl.cells.iter().take(take).chain(pl.top.iter()) {
            if !keys.contains(&c.key) {
                keys.push(c.key.clone());
            }
        }
    }
    for filter in [&filters[0], &filters[1]] {
        for key in &keys {
            let q = Query::new(idx, &scoped, filter);
            if let Ok(d) = place_detail(&q, &photos, key) {
                out.push(serde_json::json!({ "place": { "key": key, "filter": filter, "scope": lib }, "reply": d }));
            }
        }
    }
    // the three main sections under a place filter, for the three biggest cells at zooms 3 to 5
    for zoom in [3u8, 4, 5] {
        let q = Query::new(idx, &scoped, &base);
        let pl = snapshot(&q, &photos, &Wanted { sections: &[StatsSection::Places], places: Some(PlacesView { zoom, bbox: None, heat: None }) }).places.unwrap();
        for c in pl.top.iter().take(3) {
            let sections = vec![StatsSection::Overview, StatsSection::Cameras, StatsSection::Lenses, StatsSection::Exposure, StatsSection::Time];
            let request = StatsRequest { scope: lib.clone(), filter: StatsFilter { place: Some(vec![c.key.clone()]), ..base.clone() }, sections, year_mode: None, etag: None, places: None };
            out.push(reply_of(&request, out.len()));
        }
    }
    out
}

/// Rewrites `app/harness/stats-fixtures/{gt,big}.json` with the real aggregation's replies over the
/// ground-truth library (`gt`, 400 photos) and over 117 copies of it (`big`, about 47,000 photos): the
/// Statistics harness serves these instead of a hand-written mock.
/// `cargo +1.98.0 test --locked --release dump_harness_fixtures -- --ignored --nocapture`
#[test]
#[ignore]
fn dump_harness_fixtures() {
    let dir = std::path::Path::new(GROUND_TRUTH);
    let text = std::fs::read_to_string(dir.join("manifest.csv")).expect("ground-truth manifest");
    let mut lines = text.lines();
    let head = csv_line(lines.next().unwrap());
    let manifest: Vec<Vec<String>> = lines.filter(|l| !l.is_empty()).map(csv_line).collect();
    let root = "/photos-root";
    let base = gt_rows(&manifest, &head, root);
    let library = Library { roots: vec![root.into()], hidden: vec![] };
    let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../harness/stats-fixtures");
    std::fs::create_dir_all(&out_dir).unwrap();
    for (name, copies) in [("gt", 1usize), ("big", 117)] {
        let mut id = 0i64;
        let mut parsed = vec![];
        for k in 0..copies {
            for (path, meta, dt, off) in &base {
                id += 1;
                let path = if k == 0 { path.clone() } else { path.replacen('.', &format!("-{k}."), 1).replace("/photos-root-", "/photos-root.") };
                let row = RawRow { variant_id: id, path, capture_time_ns: None, capture_time_raw: dt.clone(), timezone_offset_seconds: *off, metadata: Some(meta.clone()) };
                parsed.push(parse_row(&row).unwrap());
            }
        }
        let idx = Index::build(parsed, &[]);
        let handle = |p: &str| {
            let h = p.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
            ((1u64 << 40) + h % 1_000_000_000).to_string()
        };
        let mut replies = vec![];
        for request in fixture_requests(&idx, &library) {
            let scoped = resolve_scope(&idx, &request.scope, &library).unwrap();
            let q = Query::new(&idx, &scoped, &request.filter);
            let s = snapshot(&q, &Photos { index: &idx, id: &handle }, &Wanted { sections: &request.sections, places: request.places.clone() });
            let (indexed, total, _) = scope_counts(&idx, &scoped);
            let reply = StatsResponse {
                etag: format!("fixture-{}", replies.len()),
                generated_at: "2026-11-21T00:00:00+00:00".into(),
                coverage: Coverage { indexed, total: Some(total), pending: 0, roots: 1, pending_roots: vec![], paused: false, max_age_sec: 0, state: CoverageState::Ready },
                defs: defs(),
                overview: s.overview,
                cameras: s.cameras,
                lenses: s.lenses,
                exposure: s.exposure,
                time: s.time,
                places: s.places,
                top: s.top,
            };
            replies.push(serde_json::json!({ "request": request, "reply": StatsReply::Full(Box::new(reply)) }));
        }
        replies.extend(place_fixtures(&idx, &library, &handle));
        replies.extend(top_fixtures(&idx, &library, &handle));
        let path = out_dir.join(format!("{name}.json"));
        std::fs::write(&path, serde_json::to_string(&replies).unwrap()).unwrap();
        println!("{name}: {} replies, {} photos, {} KB", replies.len(), idx.rows.len(), std::fs::metadata(&path).unwrap().len() / 1024);
    }
}
