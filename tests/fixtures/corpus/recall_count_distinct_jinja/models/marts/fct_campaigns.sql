select count(distinct {{ adapter.quote('user_id') }}) from events
