select a.id, b.id
from {{ ref('stg_a') }} a, {{ ref('stg_b') }} b
where {{ broken jinja syntax!!!
