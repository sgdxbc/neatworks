use boson::{index_secret, Clock};
use plonky2::plonk::circuit_data::CircuitConfig;
// use plonky2_maybe_rayon::rayon;
use tracing::info;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let config = CircuitConfig {
        zero_knowledge: true,
        ..CircuitConfig::standard_ecc_config()
    };

    // info!(
    //     "Using {} compute threads on {} cores",
    //     rayon::current_num_threads(),
    //     16
    // );

    const S: usize = 4;
    let (clock, circuit) = Clock::<S>::genesis(
        [(); S].map({
            let mut i = 0;
            move |()| {
                let secret = index_secret(i);
                i += 1;
                boson::public_key(secret)
            }
        }),
        config,
    )?;
    // let clock_bytes =
    //     std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("genesis_clock4.bin"))?;
    // let circuit_bytes = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("circuit4.bin"))?;
    // let (clock, circuit) = Clock::<S>::from_bytes(clock_bytes, &circuit_bytes, config)?;

    clock.verify(&circuit)?;

    // let clock10 = clock.increment(0, index_secret(0), &circuit)?;
    // let clock11 = clock.increment(1, index_secret(1), &circuit)?;
    // let clock2 = clock10.merge(&clock11, &circuit)?;

    let mut clocks = Vec::new();
    for index in 0..4 {
        clocks.push(clock.clone());
        // for i in 0..max_depth {
        for i in 0..10 {
            let clock =
                clocks
                    .last()
                    .as_ref()
                    .unwrap()
                    .increment(index, index_secret(index), &circuit)?;
            clock.verify(&circuit)?;
            let compressed = clock.proof.clone().compress(
                &circuit.data.verifier_only.circuit_digest,
                &circuit.data.common,
            )?;
            if index == 0 {
                info!(
                    "depth {i} proof size {} compressed {}",
                    clock.proof.to_bytes().len(),
                    compressed.to_bytes().len()
                )
            }
            clocks.push(clock)
        }
    }
    for _ in 0..32 {
        use rand::seq::SliceRandom;
        let clock1 = clocks.choose(&mut rand::thread_rng()).unwrap();
        let clock2 = clocks.choose(&mut rand::thread_rng()).unwrap();
        // info!(
        //     "merge {:?} and {:?}",
        //     clock1.counters().collect::<Vec<_>>(),
        //     clock2.counters().collect::<Vec<_>>(),
        // );
        let clock = clock1.merge(clock2, &circuit)?;
        // info!("merged into {:?}", clock.counters().collect::<Vec<_>>());
        clock.verify(&circuit)?;
        clocks.push(clock)
    }

    Ok(())
}

pub fn parse_hex_u64(src: &str) -> anyhow::Result<u64> {
    let src = src.strip_prefix("0x").unwrap_or(src);
    u64::from_str_radix(src, 16).map_err(Into::into)
}

// fn parse_range_usize(src: &str) -> anyhow::Result<RangeInclusive<usize>> {
//     if let Some((left, right)) = src.split_once("..=") {
//         Ok(RangeInclusive::new(
//             usize::from_str(left)?,
//             usize::from_str(right)?,
//         ))
//     } else if let Some((left, right)) = src.split_once("..") {
//         Ok(RangeInclusive::new(
//             usize::from_str(left)?,
//             if right.is_empty() {
//                 usize::MAX
//             } else {
//                 usize::from_str(right)?.saturating_sub(1)
//             },
//         ))
//     } else {
//         let value = usize::from_str(src)?;
//         Ok(RangeInclusive::new(value, value))
//     }
// }
