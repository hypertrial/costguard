select lower(trim(email)), lower(trim(email)) from {{ ref('stg_users') }}
