use super::*;
use soroban_sdk::{
    testutils::{Address as _, Events},
    Address, Env, String,
};
const PER_TX: i128 = 200_000_000; // 20 XLM
const DAILY: i128 = 1_000_000_000; // 100 XLM
const AUTO: i128 = 50_000_000; // 5 XLM
const TTL_SECS: u64 = 86_400; // 24h

fn create_contract(env: &Env) -> (Address, Address, Address) {
    let owner = Address::generate(env);
    let agent = Address::generate(env);
    let contract_id = env.register(
        GuardContract,
        (owner.clone(), agent.clone(), PER_TX, DAILY, AUTO, TTL_SECS),
    );
    (owner, agent, contract_id)
}

fn read_owner(env: &Env, contract_id: &Address) -> Address {
    env.as_contract(contract_id, || {
        env.storage().instance().get(&DataKey::Owner).unwrap()
    })
}

fn read_agent(env: &Env, contract_id: &Address) -> Address {
    env.as_contract(contract_id, || {
        env.storage().instance().get(&DataKey::Agent).unwrap()
    })
}

fn read_policy(env: &Env, contract_id: &Address) -> Policy {
    env.as_contract(contract_id, || {
        env.storage().instance().get(&DataKey::Policy).unwrap()
    })
}

fn read_next_id(env: &Env, contract_id: &Address) -> u64 {
    env.as_contract(contract_id, || {
        env.storage().instance().get(&DataKey::NextId).unwrap()
    })
}

fn read_allowed(env: &Env, contract_id: &Address, recipient: &Address) -> bool {
    let recipient = recipient.clone();
    env.as_contract(contract_id, || {
        env.storage()
            .persistent()
            .get(&DataKey::Allowlist(recipient))
            .unwrap_or(false)
    })
}

// Test 1 of 10 (Stage 1)
#[test]
fn policy_fields_are_stored_as_given() {
    let policy = Policy {
        per_tx_limit: 200_000_000,
        daily_limit: 1_000_000_000,
        auto_approve_limit: 50_000_000,
        approval_ttl_secs: 86_400,
    };

    assert_eq!(policy.per_tx_limit, 200_000_000);
    assert_eq!(policy.daily_limit, 1_000_000_000);
    assert_eq!(policy.auto_approve_limit, 50_000_000);
    assert_eq!(policy.approval_ttl_secs, 86_400);
    assert_eq!(policy.clone(), policy);
}

// Test 2 of 10 (Stage 1)
#[test]
fn payment_request_starts_pending_and_uncounted() {
    let env = Env::default();
    let agent = Address::generate(&env);
    let recipient = Address::generate(&env);

    let request = PaymentRequest {
        id: 1,
        agent: agent.clone(),
        recipient: recipient.clone(),
        amount: 100_000_000,
        reason: String::from_str(&env, "Pay invoice"),
        status: Status::Pending,
        reject_reason: RejectReason::None,
        created_at: 0,
        expires_at: 0,
        counted_day: 0,
        counted_amount: 0,
        attested_not_executed: false,
        tx_hash: None,
    };

    assert_eq!(request.status, Status::Pending);
    assert_eq!(request.reject_reason, RejectReason::None);
    assert_eq!(request.counted_amount, 0);
    assert_eq!(request.tx_hash, None);
}

// Test 3 of 10 (Stage 2)
#[test]
fn event_structs_carry_their_fields() {
    let env = Env::default();
    let agent = Address::generate(&env);
    let recipient = Address::generate(&env);

    let submitted = RequestSubmitted {
        request_id: 7,
        agent: agent.clone(),
        recipient: recipient.clone(),
        amount: 100_000_000,
        status: Status::AutoApproved,
    };
    assert_eq!(submitted.request_id, 7);
    assert_eq!(submitted.status, Status::AutoApproved);
    assert_eq!(submitted.clone(), submitted);

    let rejected = RequestRejected {
        request_id: 8,
        reason: RejectReason::PerTxLimitExceeded,
        amount: 900_000_000,
    };
    assert_eq!(rejected.reason, RejectReason::PerTxLimitExceeded);
}

// Test 4 of 10 (Stage 2)
#[test]
fn event_can_be_published_in_contract_context() {
    let env = Env::default();
    let (_owner, _agent, contract_id) = create_contract(&env);
    let agent = Address::generate(&env);
    let recipient = Address::generate(&env);

    env.as_contract(&contract_id, || {
        RequestSubmitted {
            request_id: 1,
            agent,
            recipient,
            amount: 100_000_000,
            status: Status::Pending,
        }
        .publish(&env);
    });

    // One event from this publish, plus the constructor's PolicySet event.
    assert_eq!(env.events().all().events().len(), 1);
}// --- Stage 3 tests ----------------------------------------------------------

// Test 5 of 10
#[test]
fn constructor_stores_owner_agent_policy_and_next_id() {
    let env = Env::default();
    let (owner, agent, contract_id) = create_contract(&env);

    assert_eq!(read_owner(&env, &contract_id), owner);
    assert_eq!(read_agent(&env, &contract_id), agent);
    assert_eq!(
        read_policy(&env, &contract_id),
        Policy {
            per_tx_limit: PER_TX,
            daily_limit: DAILY,
            auto_approve_limit: AUTO,
            approval_ttl_secs: TTL_SECS,
        }
    );
    assert_eq!(read_next_id(&env, &contract_id), 1);
}

// Test 6 of 10
#[test]
#[should_panic]
fn constructor_rejects_invalid_policy() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let agent = Address::generate(&env);

    // auto_approve_limit (300) > per_tx_limit (200): invalid.
    env.register(
        GuardContract,
        (owner, agent, 200_i128, 1_000_i128, 300_i128, 86_400_u64),
    );
}

// Test 7 of 10
#[test]
fn set_policy_updates_policy_and_rejects_invalid_values() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _agent, contract_id) = create_contract(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    client.set_policy(&owner, &300_000_000, &2_000_000_000, &100_000_000, &3_600);

    assert_eq!(
        read_policy(&env, &contract_id),
        Policy {
            per_tx_limit: 300_000_000,
            daily_limit: 2_000_000_000,
            auto_approve_limit: 100_000_000,
            approval_ttl_secs: 3_600,
        }
    );

    // auto (400) > per_tx (300): must fail with InvalidPolicy.
    assert_eq!(
        client.try_set_policy(
            &owner,
            &300_000_000,
            &2_000_000_000,
            &400_000_000,
            &3_600
        ),
        Err(Ok(Error::InvalidPolicy))
    );
}

// Test 8 of 10
#[test]
fn set_policy_rejects_non_owner() {
    let env = Env::default();
    env.mock_all_auths();
    let (_owner, _agent, contract_id) = create_contract(&env);
    let intruder = Address::generate(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    assert_eq!(
        client.try_set_policy(
            &intruder,
            &300_000_000,
            &2_000_000_000,
            &100_000_000,
            &3_600
        ),
        Err(Ok(Error::Unauthorized))
    );
}

// Test 9 of 10
#[test]
fn set_agent_rotates_agent_and_rejects_non_owner() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _agent, contract_id) = create_contract(&env);
    let new_agent = Address::generate(&env);
    let intruder = Address::generate(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    client.set_agent(&owner, &new_agent);
    assert_eq!(read_agent(&env, &contract_id), new_agent);

    assert_eq!(
        client.try_set_agent(&intruder, &intruder),
        Err(Ok(Error::Unauthorized))
    );
}

// Test 10 of 10
#[test]
fn add_and_remove_recipient_updates_allowlist() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _agent, contract_id) = create_contract(&env);
    let recipient = Address::generate(&env);
    let intruder = Address::generate(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    // Empty allowlist: nobody is authorized.
    assert!(!read_allowed(&env, &contract_id, &recipient));

    client.add_recipient(&owner, &recipient);
    assert!(read_allowed(&env, &contract_id, &recipient));

    client.remove_recipient(&owner, &recipient);
    assert!(!read_allowed(&env, &contract_id, &recipient));

    assert_eq!(
        client.try_add_recipient(&intruder, &recipient),
        Err(Ok(Error::Unauthorized))
    );
}