with model_count as (
    select count(*) as count_a from {{ ref('stg_events') }}
),
sources_count as (
    select sum(count_b) as count_b
    from (
        select count(*) as count_b
        from {{ ref('stg_events') }}
    ) b
)
select * from model_count
