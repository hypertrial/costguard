select * from {{ ref('stg_a') }} a join {{ ref('stg_b') }} b on a.name rlike b.token
