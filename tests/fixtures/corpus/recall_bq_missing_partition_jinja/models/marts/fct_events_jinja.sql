select * from {{ ref('stg_events') }} where status = 'active'
