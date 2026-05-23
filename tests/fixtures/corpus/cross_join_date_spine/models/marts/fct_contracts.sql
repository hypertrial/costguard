with check_date as (
    select cast('2024-01-01' as timestamp) as base_time
)
select t.hash
from {{ ref('stg_transactions') }} t
cross join check_date cd
