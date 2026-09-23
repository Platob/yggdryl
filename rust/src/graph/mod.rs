//! The graph vocabulary: what an element of a graph answers about itself,
//! and one walk over elements.
//!
//! A graph is elements that know their own identity and, where they follow
//! one, the identity of the element before them, and [`element`] holds the
//! four traits that state it. [`Element`] is the node: its
//! [`Uuid`](crate::Uuid), the one it has elsewhere, the names it goes by
//! and the UUIDs of what it was read from, read and written, the order it
//! stands in, and how it follows and merges.
//! [`Event`] is an element that also happened at one instant and stands in
//! one state. [`MarketElement`] is one that stands in a market: a price, a
//! quantity and a side; [`MarketEvent`] is one that did both. The traits
//! state signatures and the provided readings - no storage - so a message,
//! a chain entry and a lifecycle incarnation can each be an element without
//! the graph owning any of them; [`MarketElementData`] and
//! [`MarketEventData`] hold the facts as plain fields for the holder that
//! wants nothing more. The one walk, [`EventIterator`], reads events in
//! their order and states each as the one after the live element it
//! follows. [`EventColumn`] is the seventeen columns every generated schema
//! of an event states - one per fact the traits answer, under one name and
//! one datatype each - so a text line's batch, a FIX row and a chained
//! message join on them without a mapping. Event-native schemas use
//! [`EventColumn::ALL`] order; a FIX row keeps its protocol-oriented bands.

macro_rules! delegate_market_value {
    ($type:ty, $holder:ty, element) => {
        delegate_market_value!($type, $holder, element_only);
        delegate_market_value!($type, $holder, market_only);
    };
    ($type:ty, $holder:ty, element_only) => {
        impl $crate::graph::Element for $type {
            fn get_curruuid(&self) -> $crate::Uuid {
                <$holder as $crate::graph::Element>::get_curruuid(<Self as AsRef<$holder>>::as_ref(
                    self,
                ))
            }

            fn set_curruuid(&mut self, curruuid: $crate::Uuid) {
                <$holder as $crate::graph::Element>::set_curruuid(
                    <Self as AsMut<$holder>>::as_mut(self),
                    curruuid,
                );
            }

            fn get_crossuuid(&self) -> $crate::Uuid {
                <$holder as $crate::graph::Element>::get_crossuuid(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_crossuuid(&mut self, crossuuid: $crate::Uuid) {
                <$holder as $crate::graph::Element>::set_crossuuid(
                    <Self as AsMut<$holder>>::as_mut(self),
                    crossuuid,
                );
            }

            fn get_crosscode(&self) -> &str {
                <$holder as $crate::graph::Element>::get_crosscode(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_crosscode(&mut self, crosscode: String) {
                <$holder as $crate::graph::Element>::set_crosscode(
                    <Self as AsMut<$holder>>::as_mut(self),
                    crosscode,
                );
            }

            fn get_currhashcode(&self) -> u64 {
                <$holder as $crate::graph::Element>::get_currhashcode(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_currhashcode(&mut self, hashcode: u64) {
                <$holder as $crate::graph::Element>::set_currhashcode(
                    <Self as AsMut<$holder>>::as_mut(self),
                    hashcode,
                );
            }

            fn get_crosshashcode(&self) -> u64 {
                <$holder as $crate::graph::Element>::get_crosshashcode(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_crosshashcode(&mut self, crosshashcode: u64) {
                <$holder as $crate::graph::Element>::set_crosshashcode(
                    <Self as AsMut<$holder>>::as_mut(self),
                    crosshashcode,
                );
            }

            fn get_identifiers(&self) -> &std::collections::BTreeMap<String, String> {
                <$holder as $crate::graph::Element>::get_identifiers(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_identifiers(&mut self, identifiers: std::collections::BTreeMap<String, String>) {
                <$holder as $crate::graph::Element>::set_identifiers(
                    <Self as AsMut<$holder>>::as_mut(self),
                    identifiers,
                );
            }

            fn get_srcuuids(&self) -> &[$crate::Uuid] {
                <$holder as $crate::graph::Element>::get_srcuuids(<Self as AsRef<$holder>>::as_ref(
                    self,
                ))
            }

            fn set_srcuuids(&mut self, sources: Vec<$crate::Uuid>) {
                <$holder as $crate::graph::Element>::set_srcuuids(
                    <Self as AsMut<$holder>>::as_mut(self),
                    sources,
                );
            }

            fn is_after(&self, other: &Self) -> bool {
                <$holder as $crate::graph::Element>::is_after(
                    <Self as AsRef<$holder>>::as_ref(self),
                    <Self as AsRef<$holder>>::as_ref(other),
                )
            }

            fn finalize(&mut self) {
                <$holder as $crate::graph::Element>::finalize(<Self as AsMut<$holder>>::as_mut(
                    self,
                ));
            }

            fn with_previous(mut self, previous: &Self) -> Option<Self> {
                let previous = <Self as AsRef<$holder>>::as_ref(previous);
                let holder = std::mem::take(<Self as AsMut<$holder>>::as_mut(&mut self));
                let holder = <$holder as $crate::graph::Element>::with_previous(holder, previous)?;
                *<Self as AsMut<$holder>>::as_mut(&mut self) = holder;
                Some(self)
            }

            fn merge_with(mut self, other: &Self) -> Option<Self> {
                let other = <Self as AsRef<$holder>>::as_ref(other);
                let holder = std::mem::take(<Self as AsMut<$holder>>::as_mut(&mut self));
                let holder = <$holder as $crate::graph::Element>::merge_with(holder, other)?;
                *<Self as AsMut<$holder>>::as_mut(&mut self) = holder;
                Some(self)
            }
        }
    };
    ($type:ty, $holder:ty, market_only) => {
        impl $crate::graph::MarketElement for $type {
            fn get_marketoperationid(&self) -> Option<i32> {
                <$holder as $crate::graph::MarketElement>::get_marketoperationid(<Self as AsRef<
                    $holder,
                >>::as_ref(
                    self
                ))
            }

            fn set_marketoperationid(&mut self, marketoperationid: Option<i32>) {
                <$holder as $crate::graph::MarketElement>::set_marketoperationid(
                    <Self as AsMut<$holder>>::as_mut(self),
                    marketoperationid,
                );
            }

            fn get_price(&self) -> $crate::Decimal18 {
                <$holder as $crate::graph::MarketElement>::get_price(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_price(&mut self, px: $crate::Decimal18) {
                <$holder as $crate::graph::MarketElement>::set_price(
                    <Self as AsMut<$holder>>::as_mut(self),
                    px,
                );
            }

            fn get_currency(&self) -> &$crate::Currency {
                <$holder as $crate::graph::MarketElement>::get_currency(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_currency(&mut self, currency: $crate::Currency) {
                <$holder as $crate::graph::MarketElement>::set_currency(
                    <Self as AsMut<$holder>>::as_mut(self),
                    currency,
                );
            }

            fn get_quantity(&self) -> $crate::Decimal18 {
                <$holder as $crate::graph::MarketElement>::get_quantity(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_quantity(&mut self, qty: $crate::Decimal18) {
                <$holder as $crate::graph::MarketElement>::set_quantity(
                    <Self as AsMut<$holder>>::as_mut(self),
                    qty,
                );
            }

            fn get_unit(&self) -> &str {
                <$holder as $crate::graph::MarketElement>::get_unit(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_unit(&mut self, unit: String) {
                <$holder as $crate::graph::MarketElement>::set_unit(
                    <Self as AsMut<$holder>>::as_mut(self),
                    unit,
                );
            }

            fn get_side(&self) -> &$crate::Side {
                <$holder as $crate::graph::MarketElement>::get_side(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_side(&mut self, side: $crate::Side) {
                <$holder as $crate::graph::MarketElement>::set_side(
                    <Self as AsMut<$holder>>::as_mut(self),
                    side,
                );
            }

            fn get_isincode(&self) -> Option<&$crate::IsinCode> {
                <$holder as $crate::graph::MarketElement>::get_isincode(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_isincode(&mut self, isincode: Option<$crate::IsinCode>) {
                <$holder as $crate::graph::MarketElement>::set_isincode(
                    <Self as AsMut<$holder>>::as_mut(self),
                    isincode,
                );
            }

            fn get_cusipcode(&self) -> Option<&$crate::CusipCode> {
                <$holder as $crate::graph::MarketElement>::get_cusipcode(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_cusipcode(&mut self, cusipcode: Option<$crate::CusipCode>) {
                <$holder as $crate::graph::MarketElement>::set_cusipcode(
                    <Self as AsMut<$holder>>::as_mut(self),
                    cusipcode,
                );
            }

            fn get_sedolcode(&self) -> Option<&$crate::SedolCode> {
                <$holder as $crate::graph::MarketElement>::get_sedolcode(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_sedolcode(&mut self, sedolcode: Option<$crate::SedolCode>) {
                <$holder as $crate::graph::MarketElement>::set_sedolcode(
                    <Self as AsMut<$holder>>::as_mut(self),
                    sedolcode,
                );
            }

            fn get_bloombergcode(&self) -> Option<&$crate::BloombergCode> {
                <$holder as $crate::graph::MarketElement>::get_bloombergcode(<Self as AsRef<
                    $holder,
                >>::as_ref(
                    self
                ))
            }

            fn set_bloombergcode(&mut self, bloombergcode: Option<$crate::BloombergCode>) {
                <$holder as $crate::graph::MarketElement>::set_bloombergcode(
                    <Self as AsMut<$holder>>::as_mut(self),
                    bloombergcode,
                );
            }

            fn get_figicode(&self) -> Option<&$crate::FIGICode> {
                <$holder as $crate::graph::MarketElement>::get_figicode(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_figicode(&mut self, figicode: Option<$crate::FIGICode>) {
                <$holder as $crate::graph::MarketElement>::set_figicode(
                    <Self as AsMut<$holder>>::as_mut(self),
                    figicode,
                );
            }

            fn get_cficode(&self) -> Option<&$crate::CfiCode> {
                <$holder as $crate::graph::MarketElement>::get_cficode(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_cficode(&mut self, cficode: Option<$crate::CfiCode>) {
                <$holder as $crate::graph::MarketElement>::set_cficode(
                    <Self as AsMut<$holder>>::as_mut(self),
                    cficode,
                );
            }

            fn get_miccode(&self) -> Option<&$crate::MicCode> {
                <$holder as $crate::graph::MarketElement>::get_miccode(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_miccode(&mut self, miccode: Option<$crate::MicCode>) {
                <$holder as $crate::graph::MarketElement>::set_miccode(
                    <Self as AsMut<$holder>>::as_mut(self),
                    miccode,
                );
            }

            fn get_lastpx(&self) -> Option<$crate::Decimal18> {
                <$holder as $crate::graph::MarketElement>::get_lastpx(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_lastpx(&mut self, px: Option<$crate::Decimal18>) {
                <$holder as $crate::graph::MarketElement>::set_lastpx(
                    <Self as AsMut<$holder>>::as_mut(self),
                    px,
                );
            }

            fn get_lastqty(&self) -> Option<$crate::Decimal18> {
                <$holder as $crate::graph::MarketElement>::get_lastqty(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_lastqty(&mut self, qty: Option<$crate::Decimal18>) {
                <$holder as $crate::graph::MarketElement>::set_lastqty(
                    <Self as AsMut<$holder>>::as_mut(self),
                    qty,
                );
            }

            fn get_tif(&self) -> Option<&str> {
                <$holder as $crate::graph::MarketElement>::get_tif(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_tif(&mut self, tif: Option<String>) {
                <$holder as $crate::graph::MarketElement>::set_tif(
                    <Self as AsMut<$holder>>::as_mut(self),
                    tif,
                );
            }

            fn get_tradable(&self) -> Option<bool> {
                <$holder as $crate::graph::MarketElement>::get_tradable(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_tradable(&mut self, tradable: Option<bool>) {
                <$holder as $crate::graph::MarketElement>::set_tradable(
                    <Self as AsMut<$holder>>::as_mut(self),
                    tradable,
                );
            }

            fn get_symbolticker(&self) -> Option<&str> {
                <$holder as $crate::graph::MarketElement>::get_symbolticker(<Self as AsRef<
                    $holder,
                >>::as_ref(self))
            }

            fn set_symbolticker(&mut self, ticker: Option<String>) {
                <$holder as $crate::graph::MarketElement>::set_symbolticker(
                    <Self as AsMut<$holder>>::as_mut(self),
                    ticker,
                );
            }

            fn get_avgpx(&self) -> Option<$crate::Decimal18> {
                <$holder as $crate::graph::MarketElement>::get_avgpx(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_avgpx(&mut self, px: Option<$crate::Decimal18>) {
                <$holder as $crate::graph::MarketElement>::set_avgpx(
                    <Self as AsMut<$holder>>::as_mut(self),
                    px,
                );
            }

            fn get_cumqty(&self) -> Option<$crate::Decimal18> {
                <$holder as $crate::graph::MarketElement>::get_cumqty(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_cumqty(&mut self, qty: Option<$crate::Decimal18>) {
                <$holder as $crate::graph::MarketElement>::set_cumqty(
                    <Self as AsMut<$holder>>::as_mut(self),
                    qty,
                );
            }

            fn get_leavesqty(&self) -> Option<$crate::Decimal18> {
                <$holder as $crate::graph::MarketElement>::get_leavesqty(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_leavesqty(&mut self, qty: Option<$crate::Decimal18>) {
                <$holder as $crate::graph::MarketElement>::set_leavesqty(
                    <Self as AsMut<$holder>>::as_mut(self),
                    qty,
                );
            }

            fn get_prevpx(&self) -> Option<$crate::Decimal18> {
                <$holder as $crate::graph::MarketElement>::get_prevpx(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_prevpx(&mut self, px: Option<$crate::Decimal18>) {
                <$holder as $crate::graph::MarketElement>::set_prevpx(
                    <Self as AsMut<$holder>>::as_mut(self),
                    px,
                );
            }

            fn get_prevqty(&self) -> Option<$crate::Decimal18> {
                <$holder as $crate::graph::MarketElement>::get_prevqty(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_prevqty(&mut self, qty: Option<$crate::Decimal18>) {
                <$holder as $crate::graph::MarketElement>::set_prevqty(
                    <Self as AsMut<$holder>>::as_mut(self),
                    qty,
                );
            }

            fn get_bidpx(&self) -> Option<$crate::Decimal18> {
                <$holder as $crate::graph::MarketElement>::get_bidpx(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_bidpx(&mut self, px: Option<$crate::Decimal18>) {
                <$holder as $crate::graph::MarketElement>::set_bidpx(
                    <Self as AsMut<$holder>>::as_mut(self),
                    px,
                );
            }

            fn get_bidcurrency(&self) -> Option<&$crate::Currency> {
                <$holder as $crate::graph::MarketElement>::get_bidcurrency(<Self as AsRef<
                    $holder,
                >>::as_ref(self))
            }

            fn set_bidcurrency(&mut self, currency: Option<$crate::Currency>) {
                <$holder as $crate::graph::MarketElement>::set_bidcurrency(
                    <Self as AsMut<$holder>>::as_mut(self),
                    currency,
                );
            }

            fn get_bidqty(&self) -> Option<$crate::Decimal18> {
                <$holder as $crate::graph::MarketElement>::get_bidqty(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_bidqty(&mut self, qty: Option<$crate::Decimal18>) {
                <$holder as $crate::graph::MarketElement>::set_bidqty(
                    <Self as AsMut<$holder>>::as_mut(self),
                    qty,
                );
            }

            fn get_bidunit(&self) -> Option<&str> {
                <$holder as $crate::graph::MarketElement>::get_bidunit(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_bidunit(&mut self, unit: Option<String>) {
                <$holder as $crate::graph::MarketElement>::set_bidunit(
                    <Self as AsMut<$holder>>::as_mut(self),
                    unit,
                );
            }

            fn get_askpx(&self) -> Option<$crate::Decimal18> {
                <$holder as $crate::graph::MarketElement>::get_askpx(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_askpx(&mut self, px: Option<$crate::Decimal18>) {
                <$holder as $crate::graph::MarketElement>::set_askpx(
                    <Self as AsMut<$holder>>::as_mut(self),
                    px,
                );
            }

            fn get_askcurrency(&self) -> Option<&$crate::Currency> {
                <$holder as $crate::graph::MarketElement>::get_askcurrency(<Self as AsRef<
                    $holder,
                >>::as_ref(self))
            }

            fn set_askcurrency(&mut self, currency: Option<$crate::Currency>) {
                <$holder as $crate::graph::MarketElement>::set_askcurrency(
                    <Self as AsMut<$holder>>::as_mut(self),
                    currency,
                );
            }

            fn get_askqty(&self) -> Option<$crate::Decimal18> {
                <$holder as $crate::graph::MarketElement>::get_askqty(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_askqty(&mut self, qty: Option<$crate::Decimal18>) {
                <$holder as $crate::graph::MarketElement>::set_askqty(
                    <Self as AsMut<$holder>>::as_mut(self),
                    qty,
                );
            }

            fn get_askunit(&self) -> Option<&str> {
                <$holder as $crate::graph::MarketElement>::get_askunit(
                    <Self as AsRef<$holder>>::as_ref(self),
                )
            }

            fn set_askunit(&mut self, unit: Option<String>) {
                <$holder as $crate::graph::MarketElement>::set_askunit(
                    <Self as AsMut<$holder>>::as_mut(self),
                    unit,
                );
            }
        }
    };
    ($type:ty, $holder:ty, event, $is_execution:expr) => {
        delegate_market_value!($type, $holder, element);
        delegate_market_value!(
            $type,
            $holder,
            @event_impl,
            $is_execution,
            |this: &mut $type, unix: i64| {
                <$holder as $crate::graph::Event>::set_currunix(
                    <Self as AsMut<$holder>>::as_mut(this),
                    unix,
                );
            },
            |_: &mut $type| {}
        );
    };
    ($type:ty, $holder:ty, event_only, $is_execution:expr) => {
        delegate_market_value!(
            $type,
            $holder,
            @event_impl,
            $is_execution,
            |this: &mut $type, unix: i64| {
                <$holder as $crate::graph::Event>::set_currunix(
                    <Self as AsMut<$holder>>::as_mut(this),
                    unix,
                );
            },
            |this: &mut $type| <$type as $crate::graph::Element>::finalize(this)
        );
    };
    ($type:ty, $holder:ty, event_only, $is_execution:expr, $set_currunix:expr) => {
        delegate_market_value!(
            $type,
            $holder,
            @event_impl,
            $is_execution,
            $set_currunix,
            |this: &mut $type| <$type as $crate::graph::Element>::finalize(this)
        );
    };
    (
        $type:ty,
        $holder:ty,
        @event_impl,
        $is_execution:expr,
        $set_currunix:expr,
        $finish_restatement:expr
    ) => {
        impl $crate::graph::Event for $type {
            fn get_currunix(&self) -> i64 {
                <$holder as $crate::graph::Event>::get_currunix(<Self as AsRef<$holder>>::as_ref(
                    self,
                ))
            }

            fn set_currunix(&mut self, unix: i64) {
                ($set_currunix)(self, unix);
            }

            fn get_state(&self) -> &$crate::State {
                <$holder as $crate::graph::Event>::get_state(<Self as AsRef<$holder>>::as_ref(self))
            }

            fn set_state(&mut self, state: $crate::State) {
                <$holder as $crate::graph::Event>::set_state(
                    <Self as AsMut<$holder>>::as_mut(self),
                    state,
                );
            }

            fn is_execution(&self) -> bool {
                ($is_execution)(self)
            }

            fn get_seqnum(&self) -> u64 {
                <$holder as $crate::graph::Event>::get_seqnum(<Self as AsRef<$holder>>::as_ref(
                    self,
                ))
            }

            fn set_seqnum(&mut self, seqnum: u64) {
                <$holder as $crate::graph::Event>::set_seqnum(
                    <Self as AsMut<$holder>>::as_mut(self),
                    seqnum,
                );
            }

            fn get_creaunix(&self) -> Option<i64> {
                <$holder as $crate::graph::Event>::get_creaunix(<Self as AsRef<$holder>>::as_ref(
                    self,
                ))
            }

            fn set_creaunix(&mut self, unix: Option<i64>) {
                <$holder as $crate::graph::Event>::set_creaunix(
                    <Self as AsMut<$holder>>::as_mut(self),
                    unix,
                );
            }

            fn get_execunix(&self) -> Option<i64> {
                <$holder as $crate::graph::Event>::get_execunix(<Self as AsRef<$holder>>::as_ref(
                    self,
                ))
            }

            fn set_execunix(&mut self, unix: Option<i64>) {
                <$holder as $crate::graph::Event>::set_execunix(
                    <Self as AsMut<$holder>>::as_mut(self),
                    unix,
                );
            }

            fn get_recdunix(&self) -> Option<i64> {
                <$holder as $crate::graph::Event>::get_recdunix(<Self as AsRef<$holder>>::as_ref(
                    self,
                ))
            }

            fn set_recdunix(&mut self, unix: Option<i64>) {
                <$holder as $crate::graph::Event>::set_recdunix(
                    <Self as AsMut<$holder>>::as_mut(self),
                    unix,
                );
            }

            fn get_exprtime(&self) -> Option<i64> {
                <$holder as $crate::graph::Event>::get_exprtime(<Self as AsRef<$holder>>::as_ref(
                    self,
                ))
            }

            fn set_exprtime(&mut self, unix: Option<i64>) {
                <$holder as $crate::graph::Event>::set_exprtime(
                    <Self as AsMut<$holder>>::as_mut(self),
                    unix,
                );
            }

            fn get_prevunix(&self) -> Option<i64> {
                <$holder as $crate::graph::Event>::get_prevunix(<Self as AsRef<$holder>>::as_ref(
                    self,
                ))
            }

            fn set_prevunix(&mut self, unix: Option<i64>) {
                <$holder as $crate::graph::Event>::set_prevunix(
                    <Self as AsMut<$holder>>::as_mut(self),
                    unix,
                );
            }

            fn get_prevuuid(&self) -> Option<$crate::Uuid> {
                <$holder as $crate::graph::Event>::get_prevuuid(<Self as AsRef<$holder>>::as_ref(
                    self,
                ))
            }

            fn set_prevuuid(&mut self, uuid: Option<$crate::Uuid>) {
                <$holder as $crate::graph::Event>::set_prevuuid(
                    <Self as AsMut<$holder>>::as_mut(self),
                    uuid,
                );
            }

            fn get_snapunix(&self) -> Option<i64> {
                <$holder as $crate::graph::Event>::get_snapunix(<Self as AsRef<$holder>>::as_ref(
                    self,
                ))
            }

            fn set_snapunix(&mut self, unix: Option<i64>) {
                <$holder as $crate::graph::Event>::set_snapunix(
                    <Self as AsMut<$holder>>::as_mut(self),
                    unix,
                );
            }

            fn restating(mut self, live: &Self) -> Self {
                let live = <Self as AsRef<$holder>>::as_ref(live);
                let holder = std::mem::take(<Self as AsMut<$holder>>::as_mut(&mut self));
                let holder = <$holder as $crate::graph::Event>::restating(holder, live);
                *<Self as AsMut<$holder>>::as_mut(&mut self) = holder;
                ($finish_restatement)(&mut self);
                self
            }

            fn finalized(&mut self, hashcode: u64) {
                <$holder as $crate::graph::Event>::finalized(
                    <Self as AsMut<$holder>>::as_mut(self),
                    hashcode,
                );
            }
        }
    };
}

macro_rules! delegate_market_element {
    ($type:ty, existing) => {
        delegate_market_value!($type, $crate::graph::MarketElementData, element);
    };
    ($type:ty, $field:ident) => {
        impl AsRef<$crate::graph::MarketElementData> for $type {
            fn as_ref(&self) -> &$crate::graph::MarketElementData {
                &self.$field
            }
        }

        impl AsMut<$crate::graph::MarketElementData> for $type {
            fn as_mut(&mut self) -> &mut $crate::graph::MarketElementData {
                &mut self.$field
            }
        }

        delegate_market_value!($type, $crate::graph::MarketElementData, element);
    };
    ($type:ty, $field:ident, market_only) => {
        delegate_market_value!($type, $crate::graph::MarketElementData, market_only);
    };
}

macro_rules! delegate_market_event {
    ($type:ty, existing, $is_execution:expr) => {
        delegate_market_value!(
            $type,
            $crate::graph::MarketEventData,
            event,
            $is_execution
        );
    };
    ($type:ty, $field:ident) => {
        delegate_market_event!(
            @impl $type,
            $field,
            |this: &Self| <$crate::graph::MarketEventData as $crate::graph::Event>::is_execution(
                &this.$field,
            )
        );
    };
    ($type:ty, $field:ident, market_only) => {
        delegate_market_value!(
            $type,
            $crate::graph::MarketEventData,
            market_only
        );
    };
    ($type:ty, $field:ident, event_only, $is_execution:expr) => {
        delegate_market_value!(
            $type,
            $crate::graph::MarketEventData,
            event_only,
            |_: &Self| $is_execution
        );
    };
    ($type:ty, $field:ident, event_only, $is_execution:expr, $set_currunix:expr) => {
        delegate_market_value!(
            $type,
            $crate::graph::MarketEventData,
            event_only,
            |_: &Self| $is_execution,
            $set_currunix
        );
    };
    ($type:ty, $field:ident, $is_execution:expr) => {
        delegate_market_event!(@impl $type, $field, |_: &Self| $is_execution);
    };
    (@impl $type:ty, $field:ident, $is_execution:expr) => {
        impl AsRef<$crate::graph::MarketEventData> for $type {
            fn as_ref(&self) -> &$crate::graph::MarketEventData {
                &self.$field
            }
        }

        impl AsMut<$crate::graph::MarketEventData> for $type {
            fn as_mut(&mut self) -> &mut $crate::graph::MarketEventData {
                &mut self.$field
            }
        }

        delegate_market_value!(
            $type,
            $crate::graph::MarketEventData,
            event,
            $is_execution
        );
    };
}

pub(super) use delegate_market_element;
pub(super) use delegate_market_event;

pub mod arrow;
pub mod book;
pub mod column;
pub mod element;
pub mod event;
pub mod execution;
pub(crate) mod instrument;
pub mod iterator;
pub mod market_column;
pub mod order;
pub mod quote;
pub mod trade;

pub use book::{
    Book, BookIterator, BookSide, GLOBAL_SYMBOL, MarketEntry, MarketOperation, MarketOperationKind,
};
pub use column::EventColumn;
pub use element::{
    Element, Event, MarketElement, MarketEntryValue, MarketEvent, MarketOperationValue,
};
pub use event::{MarketElementData, MarketEventData};
pub use execution::{Execution, ExecutionEntry};
pub use iterator::EventIterator;
pub use market_column::MarketColumn;
pub use order::{Order, OrderEntry};
pub use quote::{Quote, QuoteEntry};
pub use trade::Trade;
