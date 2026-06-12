select id from {{ ref('stg_a') }} union select id from {{ ref('stg_b') }}
