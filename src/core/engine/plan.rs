use crate::core::messages::Message;

#[derive(Clone, Copy)]
pub(crate) struct BatchConfig {
    pub(crate) window: usize,
    pub(crate) overlap: usize,
}

pub(super) fn plan_windows<'a>(
    prior: &'a [Message],
    incoming: &'a [Message],
    config: BatchConfig,
) -> Vec<Vec<&'a Message>> {
    if incoming.is_empty() {
        return Vec::new();
    }
    let stride = config.window - config.overlap;
    let prefix = if incoming.len() >= config.window {
        config.overlap
    } else {
        config.window - incoming.len()
    };
    let prefix = prefix.min(prior.len());
    let mut sequence: Vec<&Message> = prior[prior.len() - prefix..].iter().collect();
    sequence.extend(incoming);
    if sequence.len() <= config.window {
        return vec![sequence];
    }
    let mut windows = Vec::new();
    let mut start = 0;
    loop {
        let end = start + config.window;
        windows.push(sequence[start..end].to_vec());
        if end == sequence.len() {
            break;
        }
        let mut next = start + stride;
        if next + config.window > sequence.len() {
            next = sequence.len() - config.window;
        }
        if next <= start {
            break;
        }
        start = next;
    }
    windows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(row_id: i64) -> Message {
        Message {
            row_id,
            room_id: "r".to_owned(),
            user_id: "u".to_owned(),
            text: "t".to_owned(),
        }
    }

    fn row_ids(windows: &[Vec<&Message>]) -> Vec<Vec<i64>> {
        windows
            .iter()
            .map(|window| window.iter().map(|message| message.row_id).collect())
            .collect()
    }

    #[test]
    fn a_short_batch_uses_one_window_of_prior_and_incoming() {
        let prior: Vec<Message> = (0..100).map(message).collect();
        let incoming: Vec<Message> = (100..105).map(message).collect();
        let windows = plan_windows(
            &prior,
            &incoming,
            BatchConfig {
                window: 30,
                overlap: 15,
            },
        );
        assert_eq!(row_ids(&windows), vec![(75..105).collect::<Vec<_>>()]);
    }

    #[test]
    fn a_short_history_is_not_padded() {
        let prior: Vec<Message> = (0..10).map(message).collect();
        let incoming: Vec<Message> = (10..15).map(message).collect();
        let windows = plan_windows(
            &prior,
            &incoming,
            BatchConfig {
                window: 30,
                overlap: 15,
            },
        );
        assert_eq!(row_ids(&windows), vec![(0..15).collect::<Vec<_>>()]);
    }

    #[test]
    fn a_long_batch_overlaps_and_ends_on_the_last_message() {
        let incoming: Vec<Message> = (0..50).map(message).collect();
        let windows = plan_windows(
            &[],
            &incoming,
            BatchConfig {
                window: 30,
                overlap: 15,
            },
        );
        let ids = row_ids(&windows);
        assert_eq!(
            ids,
            vec![
                (0..30).collect::<Vec<_>>(),
                (15..45).collect::<Vec<_>>(),
                (20..50).collect::<Vec<_>>(),
            ]
        );
        assert!(ids.iter().all(|window| window.len() == 30));
        let mut covered = ids.concat();
        covered.sort_unstable();
        covered.dedup();
        assert_eq!(covered, (0..50).collect::<Vec<_>>());
    }

    #[test]
    fn prior_overlap_leads_the_first_window() {
        let prior: Vec<Message> = (0..100).map(message).collect();
        let incoming: Vec<Message> = (1000..1050).map(message).collect();
        let windows = plan_windows(
            &prior,
            &incoming,
            BatchConfig {
                window: 30,
                overlap: 15,
            },
        );
        let mut first: Vec<i64> = (85..100).collect();
        first.extend(1000..1015);
        let second: Vec<i64> = (1000..1030).collect();
        let third: Vec<i64> = (1015..1045).collect();
        let fourth: Vec<i64> = (1020..1050).collect();
        assert_eq!(row_ids(&windows), vec![first, second, third, fourth]);
    }

    #[test]
    fn a_zero_overlap_still_ends_on_a_full_window() {
        let incoming: Vec<Message> = (0..50).map(message).collect();
        let windows = plan_windows(
            &[],
            &incoming,
            BatchConfig {
                window: 30,
                overlap: 0,
            },
        );
        assert_eq!(
            row_ids(&windows),
            vec![(0..30).collect::<Vec<_>>(), (20..50).collect::<Vec<_>>()]
        );
    }

    #[test]
    fn an_empty_batch_has_no_windows() {
        let prior = vec![message(1)];
        let windows = plan_windows(
            &prior,
            &[],
            BatchConfig {
                window: 30,
                overlap: 15,
            },
        );
        assert!(windows.is_empty());
    }
}
