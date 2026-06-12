select id from {{ ref('stg_events') }}
order by id
