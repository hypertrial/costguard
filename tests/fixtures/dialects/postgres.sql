select distinct on (account_id) account_id, created_at from events order by account_id, created_at desc
