use std::net::*;
use std::time::Duration;
use trust_dns_resolver::AsyncResolver;
use trust_dns_resolver::config::ResolverOpts;
use trust_dns_resolver::config::*;
use trust_dns_resolver::name_server::GenericConnection;
use trust_dns_resolver::name_server::GenericConnectionProvider;
use trust_dns_resolver::name_server::TokioRuntime;

pub const DNS_LIST: &str = ",,";

/// IP addresses for Google Public DNS
pub const DNS_IPS: &[IpAddr] = &[
    IpAddr::V4(Ipv4Addr::new(208, 67, 222, 222)),
    IpAddr::V4(Ipv4Addr::new(182, 254, 116, 116)),
    IpAddr::V4(Ipv4Addr::new(208, 67, 220, 220)),
];

pub fn default_resolver() -> AsyncResolver<GenericConnection, GenericConnectionProvider<TokioRuntime>> {
    let rc: ResolverConfig = ResolverConfig::from_parts(
        None,
        vec![],
        NameServerConfigGroup::from_ips_clear(DNS_IPS, 53, true),
    );
    let mut opts: ResolverOpts = ResolverOpts::default();
    opts.ip_strategy = LookupIpStrategy::Ipv4Only;
    opts.positive_min_ttl = Some(Duration::from_secs(3600));
    AsyncResolver::tokio(rc, opts).unwrap()
}

pub async fn dns_resolv(resolver: & AsyncResolver<GenericConnection, GenericConnectionProvider<TokioRuntime>>, host: &str) -> Option<IpAddr> {
    // On Unix/Posix systems, this will read the /etc/resolv.conf
    // let mut resolver = Resolver::from_system_conf().unwrap();

    // Lookup the IP addresses associated with a name.
    let rsp = resolver.lookup_ip(host).await;
    match rsp{
        Ok(r) => {
    // There can be many addresses associated with the name,
    //  this can return IPv4 and/or IPv6 addresses
            r.iter().next()
        },
        Err(e) => {
            error!("dns reslove err {}", &e);
            None
        },
    }
    
}
