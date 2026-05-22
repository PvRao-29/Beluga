//! Perft correctness against canonical published node counts.

use beluga_core::perft::perft;
use beluga_core::position::Position;

/// (fen, [(depth, expected_nodes), ...])
struct Case {
    fen: &'static str,
    expect: &'static [(u32, u64)],
}

const CASES: &[Case] = &[
    Case {
        // Startpos
        fen: "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        expect: &[(1, 20), (2, 400), (3, 8902), (4, 197281), (5, 4865609)],
    },
    Case {
        // Kiwipete
        fen: "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        expect: &[(1, 48), (2, 2039), (3, 97862), (4, 4085603)],
    },
    Case {
        // Position 3 (rook/pawn endgame, many EP/edge cases)
        fen: "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
        expect: &[(1, 14), (2, 191), (3, 2812), (4, 43238), (5, 674624)],
    },
    Case {
        // Position 4
        fen: "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
        expect: &[(1, 6), (2, 264), (3, 9467), (4, 422333)],
    },
    Case {
        // Position 4 mirrored
        fen: "r2q1rk1/pP1p2pp/Q4n2/bbp1p3/Np6/1B3NBn/pPPP1PPP/R3K2R b KQ - 0 1",
        expect: &[(1, 6), (2, 264), (3, 9467), (4, 422333)],
    },
    Case {
        // Position 5 (talkchess)
        fen: "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
        expect: &[(1, 44), (2, 1486), (3, 62379), (4, 2103487)],
    },
    Case {
        // Position 6
        fen: "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
        expect: &[(1, 46), (2, 2079), (3, 89890), (4, 3894594)],
    },
];

#[test]
fn perft_known_positions() {
    for case in CASES {
        let mut pos = Position::from_fen(case.fen).expect("valid fen");
        for &(depth, expected) in case.expect {
            let got = perft(&mut pos, depth);
            assert_eq!(
                got, expected,
                "perft({depth}) mismatch for {}: got {got}, want {expected}",
                case.fen
            );
        }
    }
}
