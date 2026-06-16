with pools as (
    select bpool as pools
    from "delta_prod"."b_cow_amm_arbitrum"."BCoWFactory_evt_LOG_NEW_POOL"
),
joins as (
    select p.pools as pool
    from "delta_prod"."erc20_arbitrum"."evt_Transfer" e
    inner join pools p on e."to" = p.pools
),
exits as (
    select p.pools as pool
    from "delta_prod"."erc20_arbitrum"."evt_Transfer" e
    inner join pools p on e."from" = p.pools
)
select * from joins union all select * from exits
