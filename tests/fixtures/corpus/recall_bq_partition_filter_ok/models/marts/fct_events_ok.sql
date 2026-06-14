select * from {{ ref('stg_events') }} where _PARTITIONDATE >= date_sub(current_date(), interval 3 day)
