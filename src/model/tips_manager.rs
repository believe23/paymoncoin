use std::collections::{HashMap, HashSet, LinkedList};
use std::iter::Iterator;
use std::cmp::max;
use std::sync::{Arc, Mutex};

use model::transaction::*;
use storage::hive::Hive;
use model::milestone::{Milestone, MilestoneObject};
use utils::defines::AM;
use model::tips_view_model::TipsViewModel;
use model::ledger_validator::LedgerValidator;
use model::transaction_validator::TransactionValidator;
use model::transaction_validator::TransactionError;

use rand::Rng;
use std::time::Duration;
use std::thread::JoinHandle;

extern crate rand;

#[derive(Hash, Eq, PartialEq, Debug)]
struct Pair(Hash, i64);

pub struct TipsManager {
    hive: AM<Hive>,
    max_depth: u32,
    milestone: AM<Milestone>,
    milestone_start_index: u32,
    tips_view_model: AM<TipsViewModel>,
    ledger_validator: AM<LedgerValidator>,
    transaction_validator: AM<TransactionValidator>,
    testnet: bool,
    //private int RATING_THRESHOLD = 75; // Must be in [0..100] range
    shutting_down: bool,
    //private int RESCAN_TX_TO_REQUEST_INTERVAL = 750;
    solidity_rescan_handle: Option<JoinHandle<()>>
}


impl TipsManager {
    pub fn new(hive: AM<Hive>,
               milestone: AM<Milestone>,
               ledger_validator: AM<LedgerValidator>,
               transaction_validator: AM<TransactionValidator>,
               tips_view_model: AM<TipsViewModel>,
               max_depth: u32,
               testnet: bool,
               milestone_start_index: u32) -> AM<Self> {

        let tips_manager = TipsManager {
            hive,
            max_depth,
            milestone,
            milestone_start_index,
            tips_view_model,
            ledger_validator,
            transaction_validator,
            testnet,
            shutting_down: false,
            solidity_rescan_handle: None
        };
        return make_am!(tips_manager)
    }

    pub fn transaction_to_approve(&self,
                                  visited_hashes: &mut HashSet<Hash>,
                                  diff: &mut HashMap<Address, i64>,
                                  reference: Hash,
                                  extra_tip: Hash,
                                  mut depth: u32,
                                  iterations: u32) -> Result<Option<Hash>, TransactionError> {
        if depth > self.max_depth {
            depth = self.max_depth;
        }
        if let Ok(milestone_) = self.milestone.lock() {
            if milestone_.latest_solid_subhive_milestone_index > self.milestone_start_index ||
                milestone_.latest_milestone_index == self.milestone_start_index {
                let mut ratings: HashMap<Hash, i64> = HashMap::new();
                let mut analyzed_tips: HashSet<Hash> = HashSet::new();
                let mut max_depth_ok: HashSet<Hash> = HashSet::new();
                let tip = self.entry_point(reference,
                                           extra_tip,
                                           depth);
                self.serial_update_ratings(visited_hashes,
                                           tip,
                                           &mut ratings,
                                           &mut analyzed_tips,
                                           extra_tip);
                analyzed_tips.clear();
                if let Ok(mut lv) = self.ledger_validator.lock() {
                    let update_diff_is_ok = lv.update_diff(visited_hashes, diff, tip.clone())?;
                    if update_diff_is_ok {
                        return Ok(Some(self.markov_chain_monte_carlo(visited_hashes,
                                                                  diff,
                                                                  tip,
                                                                  extra_tip,
                                                                  &mut ratings,
                                                                  iterations,
                                                                  (milestone_.latest_solid_subhive_milestone_index - (depth) * 2),
                                                                  &mut max_depth_ok)?));
                    } else {
                        error!("Update Diff error");
                    }
                }
            }
        }
        return Ok(None);
    }

    fn entry_point(&self, reference: Hash, extra_tip: Hash, depth: u32) -> Hash {
        if extra_tip == HASH_NULL {
            //trunk
            if reference != HASH_NULL {
                return reference;
            } else {
                if let Ok(mlstn) = self.milestone.lock() {
                    return mlstn.latest_solid_subhive_milestone;
                }
            }
        }

        if let Ok(milestone) = self.milestone.lock() {
            let milestone_index = max(milestone.latest_solid_subhive_milestone_index - depth - 1, 0);
            if let Ok(hive) = self.hive.lock() {

                if let Some(milestone_manager) = hive.find_closest_next_milestone(milestone_index, self.testnet, self.milestone_start_index) {
                    let hash = milestone_manager.get_hash();
                    if hash != HASH_NULL {
                        return hash;
                    }
                }
                return milestone.latest_solid_subhive_milestone;
            } else {
                panic!("broken hive mutex");
            }
        }
        return HASH_NULL;
    }

    pub fn random_walk(&self,
                       visited_hashes: &HashSet<Hash>,
                       diff: &HashMap<Address, i64>,
                       start: Hash,
                       extra_tip: Hash,
                       ratings: &mut HashMap<Hash, i64>,
                       max_depth: u32,
                       max_depth_ok: &mut HashSet<Hash>) -> Result<Hash, TransactionError> {
        let mut rnd = rand::thread_rng();
        let mut tip = start.clone();
        let mut tail = tip.clone();
        let mut tips: Vec<Hash>;
        let mut tip_set: HashSet<Hash>;
        let mut analyzed_tips: HashSet<Hash> = HashSet::new();
        let mut traversed_tails = 0;
        let mut transaction_obj = Transaction::new();
        let mut approver_index: usize;
        let mut rating_weight: f32;
        let mut walk_ratings: Vec<f32>;
        let mut my_diff = diff.clone();
        let mut my_approved_hashes = visited_hashes.clone();

        while !tip.is_null() {
            transaction_obj = Transaction::from_hash(tip.clone());
            tip_set = transaction_obj.get_approvers(&self.hive).clone();
            let check_solidity_is_ok = match self.transaction_validator.lock() {
                Ok(tv) => tv.check_solidity(transaction_obj.get_hash(), false)?,
                Err(_) => panic!("broken transaction validator mutex")
            };
            let update_diff_is_ok = match self.ledger_validator.lock() {
                Ok(mut lv) => lv.update_diff(&mut my_approved_hashes, &mut my_diff,
                                             transaction_obj.get_hash())?,
                Err(_) => panic!("broken ledger validator mutex")
            };

            if transaction_obj.get_type() == TransactionType::HashOnly {
                break;
            } else if !check_solidity_is_ok {
                break;
            } else if !update_diff_is_ok {
                break;
            } else if TipsManager::below_max_depth(transaction_obj.get_hash(),
                                                   max_depth,
                                                   max_depth_ok) {
                break;
            } else if transaction_obj.calculate_hash() == extra_tip {
                break;
            }

            tail = tip.clone();
            traversed_tails += 1;

            if tip_set.capacity() == 0 {
                break;
            } else if tip_set.capacity() == 1 {
                let mut hash_iterator = tip_set.iter();

                match hash_iterator.next() {
                    Some(hash) => {
                        tip = match tip_set.get(&hash) {
                            Some(hash) => *hash,
                            None => HASH_NULL,
                        };
                    }
                    None => tip = HASH_NULL
                }
            } else {
                tips = TipsManager::set_to_vec(&tip_set);
                if !ratings.contains_key(&tip) {
                    self.serial_update_ratings(
                        &my_approved_hashes,
                        tip,
                        ratings,
                        &mut analyzed_tips,
                        extra_tip);
                    analyzed_tips.clear();
                }

                walk_ratings = Vec::with_capacity(tips.capacity());
                let mut max_rating: f32 = 0f32;
                let mut tip_rating: i64 = match ratings.get(&tip) {
                    Some(x) => *x,
                    None => break
                };
                for i in 0..tips.capacity() {
                    walk_ratings[i] = ((tip_rating - TipsManager::get_or_default(ratings,
                                                                                 tips[i],
                                                                                 0i64)) as f32).powf(-3 as f32);
                    max_rating += walk_ratings[i];
                }

                rating_weight = rnd.gen::<f32>() * max_rating;
                approver_index = tips.capacity();
                for i in tips.capacity()..0 {
                    approver_index = i;
                    rating_weight -= walk_ratings[approver_index];
                    if rating_weight <= 0 as f32 {
                        break;
                    }
                }
                tip = tips[approver_index as usize].clone();
                if transaction_obj.calculate_hash() == tip {
                    break;
                }
            }
        }
        return Ok(tail);
    }

    pub fn markov_chain_monte_carlo(&self,
                                    visited_hashes: &HashSet<Hash>,
                                    diff: &HashMap<Address, i64>,
                                    tip: Hash,
                                    extra_tip: Hash,
                                    ratings: &mut HashMap<Hash, i64>,
                                    iterations: u32,
                                    max_depth: u32,
                                    max_depth_ok: &mut HashSet<Hash>,
                                    /*Random seed*/) -> Result<Hash, TransactionError> {
        let mut rnd = rand::thread_rng();
        let mut monte_carlo_integrations: &mut HashMap<Hash, i64> = &mut HashMap::new();
        let mut map_clone = monte_carlo_integrations.clone();
        let mut tail: Hash;
        for i in iterations..0 {
            tail = self.random_walk(visited_hashes, diff, tip, extra_tip, ratings, max_depth, max_depth_ok)?;
            if monte_carlo_integrations.contains_key(&tail) {
                let taken_from_map = match map_clone.get(&tail) {
                    Some(value) => *value,
                    None => 0i64,
                };
                TipsManager::put(monte_carlo_integrations, tail, (taken_from_map + 1));
            } else {
                TipsManager::put(monte_carlo_integrations, tail, 1);
            }
        }

        let res_set = monte_carlo_integrations.iter()
            .map(|(x, y)| Pair(*x, *y))
            .collect::<HashSet<_>>();

        return Ok(res_set.iter()
            .fold(HASH_NULL, |a, b| {
                if *monte_carlo_integrations.get(&a).unwrap() > b.1 {
                    return a;
                } else if *monte_carlo_integrations.get(&a).unwrap() < b.1 {
                    return b.0;
                } else if rnd.gen() {
                    return a;
                } else {
                    return b.0;
                }
            }));
    }

    fn set_to_vec(set: &HashSet<Hash>) -> Vec<Hash> {
        let mut hash_iterator = set.iter();
        let mut result: Vec<Hash> = Vec::new();
        if !set.is_empty() {
            loop {
                match hash_iterator.next() {
                    Some(hash) => result.push(*set.get(hash).unwrap()),
                    None => break
                }
            }
        }
        return result;
    }

    fn serial_update_ratings(&self,
                             visited_hashes: &HashSet<Hash>,
                             tx_hash: Hash,
                             ratings: &mut HashMap<Hash, i64>,
                             analyzed_tips: &mut HashSet<Hash>,
                             extra_tip: Hash) {
        let mut hashes_to_rate: LinkedList<Hash> = LinkedList::new();
        hashes_to_rate.push_front(tx_hash);
        let mut current_hash: Hash;
        let mut added_back: bool;
        while !hashes_to_rate.is_empty() {
            match hashes_to_rate.pop_front() {
                Some(hash) => current_hash = hash,
                None => {
                    println!("Stack is empty!");
                    return;
                }
            }
            let mut transaction: Transaction = Transaction::from_hash(current_hash);
            added_back = false;
            let mut approvers: HashSet<Hash> = transaction.get_approvers(&self.hive).clone();
            for approver in &approvers {
                let mut flag: bool = match ratings.get(approver) {
                    Some(..) => true,
                    None => false
                };
                if flag && *approver != current_hash {
                    if !added_back {
                        added_back = true;
                        hashes_to_rate.push_front(current_hash);
                    }
                    hashes_to_rate.push_front(*approver);
                }
            }
            if !added_back && TipsManager::add(analyzed_tips, current_hash) {
                let rating: i64 = TipsManager::rating_calc(extra_tip, &visited_hashes, current_hash, &approvers, ratings);
                TipsManager::put(ratings, current_hash, rating);
            }
        }
    }

    fn add(set: &mut HashSet<Hash>, curr: Hash) -> bool {
        match set.get(&curr) {
            Some(..) => {
                return false;
            }
            None => {
                set.insert(curr);
                return true;
            }
        }
    }

    fn rating_calc(extra_tip: Hash, visited_hashes: &HashSet<Hash>, current_hash: Hash, approvers: &HashSet<Hash>, ratings: &HashMap<Hash, i64>) -> i64 {
        let mut result: i64;
        result = match extra_tip == HASH_NULL && visited_hashes.contains(&current_hash) {
            true => 0,
            false => 1
        };

        result += approvers.iter().
            map(|x| ratings.get(x)).
            filter(|x| *x != None).
            fold(0, |a, b| cap_sum(a, *b.unwrap(), (<i64>::max_value() / 2)));
        return result;
    }

    fn put(map: &mut HashMap<Hash, i64>, key: Hash, value: i64) -> Option<i64> {
        let result: i64;
        match map.contains_key(&key) {
            true => {
                result = match map.get(&key) {
                    Some(long) => *long,
                    None => 0i64,
                };
                map.insert(key, value);
                return Some(result);
            }
            false => {
                map.insert(key, value);
                return None;
            }
        };
    }

    fn get_or_default(map: &HashMap<Hash, i64>, key: Hash, default_value: i64) -> i64 {
        let result: i64;
        result = match map.get(&key) {
            Some(x) => *x,
            None => default_value
        };
        return result;
    }

    fn below_max_depth(tip: Hash, depth: u32, max_depth_ok: &mut HashSet<Hash>) -> bool {
        //if tip is confirmed stop
        if TransactionObject::from_hash(tip).get_snapshot_index() >= depth {
            return false;
        }
        //if tip unconfirmed, check if any referenced tx is confirmed below maxDepth
        let mut non_analyzed_transactions = LinkedList::new();
        non_analyzed_transactions.push_front(tip);
        let mut analyzed_transactions: HashSet<Hash> = HashSet::new();
        let mut hash: Hash;
        while non_analyzed_transactions.front() != None {
            hash = match non_analyzed_transactions.front() {
                Some(h) => *h,
                None => break
            };
            if analyzed_transactions.insert(hash) {
                let mut transaction: Transaction = Transaction::from_hash(hash);
                //transaction.from_hash(&hash);
                if transaction.object.get_snapshot_index() != 0 && transaction.object.get_snapshot_index() < depth {
                    return true;
                }
                if transaction.object.get_snapshot_index() == 0 {
                    if max_depth_ok.contains(&hash) {
                        return true;
                    } else {
                        non_analyzed_transactions.push_back(transaction.get_trunk_transaction_hash());
                        non_analyzed_transactions.push_back(transaction.get_branch_transaction_hash());
                    }
                }
            }
        }
        max_depth_ok.insert(tip);
        return false;
    }

    pub fn recursive_update_ratings(&self,
                                    txHash: Hash,
                                    ratings: &mut HashMap<Hash, i64>,
                                    analyzed_tips: &mut HashSet<Hash>) -> i64 {
        let mut rating = 1;
        if analyzed_tips.insert(txHash) {
            let mut transaction = Transaction::from_hash(txHash);
            let mut approver_hashes = transaction.get_approvers(&self.hive);
            for approver in approver_hashes.iter() {
                rating = cap_sum(rating, TipsManager::recursive_update_ratings(self, *approver, ratings, analyzed_tips), (<i64>::max_value() / 2));
            }
            TipsManager::put(ratings, txHash, rating);
        } else {
            if ratings.contains_key(&txHash) {
                rating =  match ratings.get(&txHash) {
                    Some(x) => *x,
                    None => 0,
                };
            } else {
                rating = 0;
            }
        }
        return rating;
    }
}

fn cap_sum(a: i64, b: i64, max: i64) -> i64 {
    if a + b < 0 || a + b > max {
        return max;
    }
    return a + b;
}
