use aggcommon::sources::SourceId;
use aggserver::service::model::{CombinedSummary, ContributorSnapshot};
use criterion::{Criterion, criterion_group, criterion_main};

fn snapshot_with_levels<const N: usize>(
    source: SourceId,
    time: u64,
    bids: &[(f64, f64)],
    asks: &[(f64, f64)],
) -> ContributorSnapshot<N> {
    let mut s = ContributorSnapshot::new(source, time);
    for &(p, a) in bids {
        s.push_bid(p, a);
    }
    for &(p, a) in asks {
        s.push_ask(p, a);
    }
    s
}

fn bench_combined_summary_to_proto(c: &mut Criterion) {
    const N: usize = 10;

    // Define bid and ask levels for two sources
    let bids_source1 = (0..N).map(|i| (100.0 + i as f64, 10.0)).collect::<Vec<_>>();
    let asks_source1 = (0..N).map(|i| (200.0 + i as f64, 10.0)).collect::<Vec<_>>();

    let bids_source2 = (0..N)
        .map(|i| (100.0 + (i / 2) as f64, 5.0))
        .collect::<Vec<_>>();
    let asks_source2 = (0..N)
        .map(|i| (200.0 + (i / 2) as f64, 5.0))
        .collect::<Vec<_>>();

    // Create snapshots using snapshot_with_levels
    let source1 = snapshot_with_levels::<N>(SourceId::Bitstamp, 0, &bids_source1, &asks_source1);
    let source2 = snapshot_with_levels::<N>(SourceId::Binance, 0, &bids_source2, &asks_source2);

    let mut combined_summary = CombinedSummary::<N>::new();
    combined_summary.update_snapshot(source1);
    combined_summary.update_snapshot(source2);

    c.bench_function("combined_summary_to_proto", |b| {
        b.iter(|| {
            let _proto = combined_summary.to_proto();
        });
    });
}

criterion_group!(benches, bench_combined_summary_to_proto,);
criterion_main!(benches);
