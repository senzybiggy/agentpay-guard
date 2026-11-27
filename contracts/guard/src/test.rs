use super::*;
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger as _},
    Address, Env, String,
};

const PER_TX: i128 = 200_000_000;
const DAILY: i128 = 1_000_000_000;
const AUTO: i128 = 50_000_000;
const TTL_SECS: u64 = 86_400;

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

fn read_request(env: &Env, contract_id: &Address, request_id: u64) -> PaymentRequest {
    env.as_contract(contract_id, || {
        env.storage()
            .persistent()
            .get(&DataKey::Request(request_id))
            .unwrap()
    })
}

fn read_spent_today(env: &Env, contract_id: &Address) -> i128 {
    let today = env.ledger().timestamp() / 86_400;
    read_spent_on_day(env, contract_id, today)
}

fn read_spent_on_day(env: &Env, contract_id: &Address, day: u64) -> i128 {
    env.as_contract(contract_id, || {
        env.storage()
            .persistent()
            .get(&DataKey::SpentOnDay(day))
            .unwrap_or(0)
    })
}

fn setup_pending_request(
    env: &Env,
) -> (Address, Address, Address, Address, u64) {
    env.mock_all_auths();

    let (owner, agent, contract_id) = create_contract(env);
    let recipient = Address::generate(env);
    let client = GuardContractClient::new(env, &contract_id);

    client.add_recipient(&owner, &recipient);

    let request_id = client.submit_request(
        &agent,
        &recipient,
        &100_000_000_i128,
        &String::from_str(env, "Test payment"),
    );

    (owner, agent, contract_id, recipient, request_id)
}

fn setup_auto_approved_request(
    env: &Env,
) -> (Address, Address, Address, u64) {
    env.mock_all_auths();

    let (owner, agent, contract_id) = create_contract(env);
    let recipient = Address::generate(env);
    let client = GuardContractClient::new(env, &contract_id);

    client.add_recipient(&owner, &recipient);

    let request_id = client.submit_request(
        &agent,
        &recipient,
        &10_000_000_i128,
        &String::from_str(env, "Automatic payment"),
    );

    (owner, agent, contract_id, request_id)
}

// STAGE 1

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

#[test]
fn payment_request_starts_pending_and_uncounted() {
    let env = Env::default();
    let agent = Address::generate(&env);
    let recipient = Address::generate(&env);

    let request = PaymentRequest {
        id: 1,
        agent,
        recipient,
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

// STAGE 2

#[test]
fn event_structs_carry_their_fields() {
    let env = Env::default();
    let agent = Address::generate(&env);
    let recipient = Address::generate(&env);

    let submitted = RequestSubmitted {
        request_id: 7,
        agent,
        recipient,
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

    assert_eq!(env.events().all().events().len(), 1);
}

// STAGE 3

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

#[test]
#[should_panic]
fn constructor_rejects_invalid_policy() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let agent = Address::generate(&env);

    env.register(
        GuardContract,
        (owner, agent, 200_i128, 1_000_i128, 300_i128, 86_400_u64),
    );
}

#[test]
fn set_policy_updates_policy_and_rejects_invalid_values() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _, contract_id) = create_contract(&env);
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

#[test]
fn set_policy_rejects_non_owner() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, _, contract_id) = create_contract(&env);
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

#[test]
fn set_agent_rotates_agent_and_rejects_non_owner() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _, contract_id) = create_contract(&env);
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

#[test]
#[should_panic]
fn constructor_rejects_agent_equal_to_owner() {
    let env = Env::default();
    let owner = Address::generate(&env);

    env.register(
        GuardContract,
        (owner.clone(), owner, PER_TX, DAILY, AUTO, TTL_SECS),
    );
}

#[test]
fn set_agent_rejects_owner_as_agent() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, agent, contract_id) = create_contract(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    assert_eq!(
        client.try_set_agent(&owner, &owner),
        Err(Ok(Error::Unauthorized))
    );

    assert_eq!(read_agent(&env, &contract_id), agent);
}

#[test]
fn add_and_remove_recipient_updates_allowlist() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _, contract_id) = create_contract(&env);
    let recipient = Address::generate(&env);
    let intruder = Address::generate(&env);
    let client = GuardContractClient::new(&env, &contract_id);

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

// STAGE 4

#[test]
fn submit_request_auto_approves_and_reserves_spending() {
    let env = Env::default();
    env.mock_all_auths();

    let (owner, agent, contract_id) = create_contract(&env);
    let recipient = Address::generate(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    client.add_recipient(&owner, &recipient);

    let amount = 10_000_000_i128;
    let reason = String::from_str(&env, "Approved payment");

    let id = client.submit_request(&agent, &recipient, &amount, &reason);
    let request = read_request(&env, &contract_id, id);

    assert_eq!(id, 1);
    assert_eq!(request.status, Status::AutoApproved);
    assert_eq!(request.reject_reason, RejectReason::None);
    assert_eq!(request.counted_amount, amount);
    assert_eq!(read_spent_today(&env, &contract_id), amount);
}

#[test]
fn submit_request_above_auto_limit_remains_pending() {
    let env = Env::default();
    env.mock_all_auths();

    let (owner, agent, contract_id) = create_contract(&env);
    let recipient = Address::generate(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    client.add_recipient(&owner, &recipient);

    let amount = 100_000_000_i128;
    let reason = String::from_str(&env, "Requires owner approval");

    let id = client.submit_request(&agent, &recipient, &amount, &reason);
    let request = read_request(&env, &contract_id, id);

    assert_eq!(request.status, Status::Pending);
    assert_eq!(request.counted_amount, 0);
    assert_eq!(read_spent_today(&env, &contract_id), 0);
}

#[test]
fn submit_request_rejects_non_allowlisted_recipient() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, agent, contract_id) = create_contract(&env);
    let recipient = Address::generate(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    let amount = 10_000_000_i128;
    let reason = String::from_str(&env, "Unlisted recipient");

    let id = client.submit_request(&agent, &recipient, &amount, &reason);
    let request = read_request(&env, &contract_id, id);

    assert_eq!(request.status, Status::Rejected);
    assert_eq!(request.reject_reason, RejectReason::NotAllowlisted);
    assert_eq!(request.counted_amount, 0);
}

#[test]
fn submit_request_rejects_per_transaction_limit() {
    let env = Env::default();
    env.mock_all_auths();

    let (owner, agent, contract_id) = create_contract(&env);
    let recipient = Address::generate(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    client.add_recipient(&owner, &recipient);

    let amount = 300_000_000_i128;
    let reason = String::from_str(&env, "Above transaction limit");

    let id = client.submit_request(&agent, &recipient, &amount, &reason);
    let request = read_request(&env, &contract_id, id);

    assert_eq!(request.status, Status::Rejected);
    assert_eq!(request.reject_reason, RejectReason::PerTxLimitExceeded);
}

#[test]
fn submit_request_rejects_daily_limit_exceeded() {
    let env = Env::default();
    env.mock_all_auths();

    let (owner, agent, contract_id) = create_contract(&env);
    let recipient = Address::generate(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    client.add_recipient(&owner, &recipient);

    client.set_policy(
        &owner,
        &100_000_000,
        &100_000_000,
        &50_000_000,
        &TTL_SECS,
    );

    let reason = String::from_str(&env, "Daily spending test");
    let amount = 50_000_000_i128;

    client.submit_request(&agent, &recipient, &amount, &reason);
    client.submit_request(&agent, &recipient, &amount, &reason);

    assert_eq!(read_spent_today(&env, &contract_id), 100_000_000);

    let third_amount = 1_i128;
    let id = client.submit_request(&agent, &recipient, &third_amount, &reason);
    let request = read_request(&env, &contract_id, id);

    assert_eq!(request.status, Status::Rejected);
    assert_eq!(request.reject_reason, RejectReason::DailyLimitExceeded);
    assert_eq!(read_spent_today(&env, &contract_id), 100_000_000);
}

#[test]
fn submit_request_rejects_unauthorized_agent() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, _, contract_id) = create_contract(&env);
    let intruder = Address::generate(&env);
    let recipient = Address::generate(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    let amount = 10_000_000_i128;
    let reason = String::from_str(&env, "Unauthorized request");

    assert_eq!(
        client.try_submit_request(&intruder, &recipient, &amount, &reason),
        Err(Ok(Error::Unauthorized))
    );
}

#[test]
fn submit_request_rejects_invalid_amount() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, agent, contract_id) = create_contract(&env);
    let recipient = Address::generate(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    let amount = 0_i128;
    let reason = String::from_str(&env, "Invalid amount");

    assert_eq!(
        client.try_submit_request(&agent, &recipient, &amount, &reason),
        Err(Ok(Error::InvalidAmount))
    );

    assert_eq!(read_next_id(&env, &contract_id), 1);
}

// STAGE 5

#[test]
fn test18_approve_pending_request() {
    let env = Env::default();
    let (owner, _, contract_id, _, request_id) = setup_pending_request(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    let status = client.approve_request(&owner, &request_id);

    assert_eq!(status, Status::Approved);

    let request = read_request(&env, &contract_id, request_id);
    assert_eq!(request.status, Status::Approved);
    assert_eq!(request.counted_amount, 100_000_000);
    assert_eq!(read_spent_today(&env, &contract_id), 100_000_000);
}

#[test]
fn test19_approve_request_unauthorized_owner() {
    let env = Env::default();
    let (_, _, contract_id, _, request_id) = setup_pending_request(&env);

    let attacker = Address::generate(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    assert_eq!(
        client.try_approve_request(&attacker, &request_id),
        Err(Ok(Error::Unauthorized))
    );
}

#[test]
fn test20_approve_missing_request() {
    let env = Env::default();
    env.mock_all_auths();

    let (owner, _, contract_id) = create_contract(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    assert_eq!(
        client.try_approve_request(&owner, &999),
        Err(Ok(Error::RequestNotFound))
    );
}

#[test]
fn test21_cannot_approve_request_twice() {
    let env = Env::default();
    let (owner, _, contract_id, _, request_id) = setup_pending_request(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    client.approve_request(&owner, &request_id);

    assert_eq!(
        client.try_approve_request(&owner, &request_id),
        Err(Ok(Error::InvalidStatus))
    );

    assert_eq!(read_spent_today(&env, &contract_id), 100_000_000);
}

#[test]
fn test22_reject_pending_request() {
    let env = Env::default();
    let (owner, _, contract_id, _, request_id) = setup_pending_request(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    let status = client.reject_request(&owner, &request_id);

    assert_eq!(status, Status::Rejected);

    let request = read_request(&env, &contract_id, request_id);
    assert_eq!(request.status, Status::Rejected);
    assert_eq!(request.reject_reason, RejectReason::OwnerRejected);
    assert_eq!(read_spent_today(&env, &contract_id), 0);
}

#[test]
fn test23_reject_missing_request() {
    let env = Env::default();
    env.mock_all_auths();

    let (owner, _, contract_id) = create_contract(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    assert_eq!(
        client.try_reject_request(&owner, &999),
        Err(Ok(Error::RequestNotFound))
    );
}

#[test]
fn test24_cannot_reject_auto_approved_request() {
    let env = Env::default();
    let (owner, _, contract_id, request_id) = setup_auto_approved_request(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    assert_eq!(
        client.try_reject_request(&owner, &request_id),
        Err(Ok(Error::InvalidStatus))
    );
}

#[test]
fn test25_cannot_approve_rejected_request() {
    let env = Env::default();
    let (owner, _, contract_id, _, request_id) = setup_pending_request(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    client.reject_request(&owner, &request_id);

    assert_eq!(
        client.try_approve_request(&owner, &request_id),
        Err(Ok(Error::InvalidStatus))
    );
}

#[test]
fn test26_expired_pending_request_cannot_be_approved() {
    let env = Env::default();
    let (owner, _, contract_id, _, request_id) = setup_pending_request(&env);

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = 86_401;
    });

    let client = GuardContractClient::new(&env, &contract_id);

    assert_eq!(
        client.try_approve_request(&owner, &request_id),
        Err(Ok(Error::RequestExpired))
    );
}

#[test]
fn test27_approval_revalidates_allowlist() {
    let env = Env::default();
    let (owner, _, contract_id, recipient, request_id) = setup_pending_request(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    client.remove_recipient(&owner, &recipient);

    let status = client.approve_request(&owner, &request_id);
    assert_eq!(status, Status::Rejected);

    let request = read_request(&env, &contract_id, request_id);

    assert_eq!(request.status, Status::Rejected);
    assert_eq!(request.reject_reason, RejectReason::NotAllowlisted);
    assert_eq!(read_spent_today(&env, &contract_id), 0);
}

#[test]
fn test28_approval_revalidates_per_transaction_limit() {
    let env = Env::default();
    let (owner, _, contract_id, _, request_id) = setup_pending_request(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    client.set_policy(
        &owner,
        &50_000_000_i128,
        &1_000_000_000_i128,
        &50_000_000_i128,
        &86_400_u64,
    );

    let status = client.approve_request(&owner, &request_id);
    assert_eq!(status, Status::Rejected);

    let request = read_request(&env, &contract_id, request_id);

    assert_eq!(request.reject_reason, RejectReason::PerTxLimitExceeded);
    assert_eq!(read_spent_today(&env, &contract_id), 0);
}

#[test]
fn test29_approval_revalidates_daily_limit() {
    let env = Env::default();
    env.mock_all_auths();

    let (owner, agent, contract_id) = create_contract(&env);
    let recipient = Address::generate(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    client.add_recipient(&owner, &recipient);

    let first_id = client.submit_request(
        &agent,
        &recipient,
        &50_000_000_i128,
        &String::from_str(&env, "First payment"),
    );

    let second_id = client.submit_request(
        &agent,
        &recipient,
        &100_000_000_i128,
        &String::from_str(&env, "Second payment"),
    );

    client.set_policy(
        &owner,
        &120_000_000_i128,
        &120_000_000_i128,
        &50_000_000_i128,
        &86_400_u64,
    );

    let status = client.approve_request(&owner, &second_id);
    assert_eq!(status, Status::Rejected);

    let first_request = read_request(&env, &contract_id, first_id);
    let second_request = read_request(&env, &contract_id, second_id);

    assert_eq!(first_request.status, Status::AutoApproved);
    assert_eq!(second_request.status, Status::Rejected);
    assert_eq!(second_request.reject_reason, RejectReason::DailyLimitExceeded);
    assert_eq!(read_spent_today(&env, &contract_id), 50_000_000);
}

// STAGE 6

#[test]
fn test30_cancel_approved_request_releases_reservation() {
    let env = Env::default();
    let (owner, _, contract_id, _, request_id) = setup_pending_request(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    client.approve_request(&owner, &request_id);

    assert_eq!(read_spent_today(&env, &contract_id), 100_000_000);

    let status = client.cancel_approved(&owner, &request_id, &true);

    assert_eq!(status, Status::Cancelled);

    let request = read_request(&env, &contract_id, request_id);

    assert_eq!(request.status, Status::Cancelled);
    assert_eq!(request.attested_not_executed, true);
    assert_eq!(request.counted_amount, 100_000_000);
    assert_eq!(read_spent_today(&env, &contract_id), 0);
}

#[test]
fn test31_cancel_requires_not_executed_attestation() {
    let env = Env::default();
    let (owner, _, contract_id, _, request_id) = setup_pending_request(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    client.approve_request(&owner, &request_id);

    assert_eq!(
        client.try_cancel_approved(&owner, &request_id, &false),
        Err(Ok(Error::AttestationRequired))
    );

    assert_eq!(read_spent_today(&env, &contract_id), 100_000_000);
}

#[test]
fn test32_cannot_cancel_pending_request() {
    let env = Env::default();
    let (owner, _, contract_id, _, request_id) = setup_pending_request(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    assert_eq!(
        client.try_cancel_approved(&owner, &request_id, &true),
        Err(Ok(Error::InvalidStatus))
    );
}

#[test]
fn test33_expire_pending_request_after_deadline() {
    let env = Env::default();
    let (owner, _, contract_id, _, request_id) = setup_pending_request(&env);

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = 86_400;
    });

    let client = GuardContractClient::new(&env, &contract_id);
    let status = client.expire_request(&owner, &request_id);

    assert_eq!(status, Status::Expired);

    let request = read_request(&env, &contract_id, request_id);
    assert_eq!(request.status, Status::Expired);
    assert_eq!(read_spent_today(&env, &contract_id), 0);
}

#[test]
fn test34_cannot_expire_pending_request_before_deadline() {
    let env = Env::default();
    let (owner, _, contract_id, _, request_id) = setup_pending_request(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    assert_eq!(
        client.try_expire_request(&owner, &request_id),
        Err(Ok(Error::InvalidStatus))
    );
}

#[test]
fn test35_unauthorized_actor_cannot_expire_request() {
    let env = Env::default();
    let (_, _, contract_id, _, request_id) = setup_pending_request(&env);

    let attacker = Address::generate(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    assert_eq!(
        client.try_expire_request(&attacker, &request_id),
        Err(Ok(Error::Unauthorized))
    );
}

#[test]
fn test36_mark_executed_records_transaction_hash() {
    let env = Env::default();
    let (owner, _, contract_id, _, request_id) = setup_pending_request(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    client.approve_request(&owner, &request_id);

    let hash = String::from_str(&env, "stellar_transaction_hash_123");
    let status = client.mark_executed(&owner, &request_id, &hash);

    assert_eq!(status, Status::Executed);

    let request = read_request(&env, &contract_id, request_id);

    assert_eq!(request.status, Status::Executed);
    assert_eq!(request.tx_hash, Some(hash));
    assert_eq!(read_spent_today(&env, &contract_id), 100_000_000);
}

#[test]
fn test37_mark_executed_rejects_empty_hash() {
    let env = Env::default();
    let (owner, _, contract_id, _, request_id) = setup_pending_request(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    client.approve_request(&owner, &request_id);

    let empty_hash = String::from_str(&env, "");

    assert_eq!(
        client.try_mark_executed(&owner, &request_id, &empty_hash),
        Err(Ok(Error::InvalidTxHash))
    );
}

#[test]
fn test38_cannot_mark_pending_request_executed() {
    let env = Env::default();
    let (owner, _, contract_id, _, request_id) = setup_pending_request(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    let hash = String::from_str(&env, "transaction_hash");

    assert_eq!(
        client.try_mark_executed(&owner, &request_id, &hash),
        Err(Ok(Error::InvalidStatus))
    );
}

#[test]
fn test39_cancellation_releases_original_day_reservation() {
    let env = Env::default();
    let (owner, _, contract_id, _, request_id) = setup_pending_request(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    client.approve_request(&owner, &request_id);

    assert_eq!(read_spent_on_day(&env, &contract_id, 0), 100_000_000);

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = 86_400;
    });

    client.cancel_approved(&owner, &request_id, &true);

    assert_eq!(read_spent_on_day(&env, &contract_id, 0), 0);
    assert_eq!(read_spent_on_day(&env, &contract_id, 1), 0);
}// STAGE 7: READ-ONLY VIEW FUNCTION TESTS

#[test]
fn test40_get_policy_returns_current_policy() {
    let env = Env::default();
    let (_, _, contract_id) = create_contract(&env);

    let client = GuardContractClient::new(&env, &contract_id);
    let policy = client.get_policy();

    assert_eq!(policy.per_tx_limit, PER_TX);
    assert_eq!(policy.daily_limit, DAILY);
    assert_eq!(policy.auto_approve_limit, AUTO);
    assert_eq!(policy.approval_ttl_secs, TTL_SECS);
}

#[test]
fn test41_get_request_returns_existing_request() {
    let env = Env::default();
    let (_, _, contract_id, _, request_id) = setup_pending_request(&env);

    let client = GuardContractClient::new(&env, &contract_id);
    let request = client.get_request(&request_id);

    assert_eq!(request.id, request_id);
    assert_eq!(request.status, Status::Pending);
    assert_eq!(request.amount, 100_000_000);
}

#[test]
fn test42_get_missing_request_returns_error() {
    let env = Env::default();
    let (_, _, contract_id) = create_contract(&env);

    let client = GuardContractClient::new(&env, &contract_id);

    assert_eq!(
        client.try_get_request(&999),
        Err(Ok(Error::RequestNotFound))
    );
}

#[test]
fn test43_request_count_returns_total_requests() {
    let env = Env::default();
    let (_, _, contract_id, _, _) = setup_pending_request(&env);

    let client = GuardContractClient::new(&env, &contract_id);

    assert_eq!(client.request_count(), 1);
}

#[test]
fn test44_spent_today_and_is_allowed_return_correct_values() {
    let env = Env::default();
    let (owner, agent, contract_id) = create_contract(&env);

    let recipient = Address::generate(&env);
    let client = GuardContractClient::new(&env, &contract_id);

    env.mock_all_auths();

    assert!(!client.is_allowed(&recipient));

    client.add_recipient(&owner, &recipient);

    assert!(client.is_allowed(&recipient));
    assert_eq!(client.spent_today(), 0);

    let amount = 10_000_000_i128;
    let reason = String::from_str(&env, "View function test");

    client.submit_request(&agent, &recipient, &amount, &reason);

    assert_eq!(client.spent_today(), amount);
}