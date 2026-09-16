/**
 * Deluge.PeersWindow.js
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * What each peer has actually done, across connections.
 *
 * The Peers tab shows the connections open right now, with the speeds of the
 * moment. That cannot answer the question people actually have about a peer:
 * what has this address ever given back? A peer that takes forty gibibytes
 * over a week and sends nothing looks idle in every snapshot, because it is
 * idle in every snapshot.
 *
 * So the daemon keeps a running account, and this reads it. Two columns are
 * the point of the window:
 *
 * `Gave back` is what the address sent this daemon against what it took. It is
 * the honest measure of a peer, because it is measured on this machine's own
 * link rather than believed from what anybody announced.
 *
 * `Cross-seeds` is how many of this daemon's contents the address carries on
 * more than one of its torrents. It is worth saying plainly that this is not
 * cheating: a peer seeding the same release to two trackers uploads real bytes
 * to both swarms, most private trackers allow it, and it is exactly what this
 * daemon's own tracker rules help its operator do. It is here because it is
 * interesting and because it confirms one's own cross-seeding is working, not
 * because it is an accusation.
 */
Ext.ns('Deluge');

/**
 * @class Deluge.PeersWindow
 * @extends Ext.Window
 */
Deluge.PeersWindow = Ext.extend(Ext.Window, {
    title: _('Peers'),
    width: 860,
    height: 460,
    layout: 'fit',
    closeAction: 'hide',
    constrainHeader: true,
    plain: true,
    minWidth: 520,
    minHeight: 300,

    /** How often the window refreshes itself while it is open, in seconds. */
    interval: 30,

    initComponent: function () {
        Deluge.PeersWindow.superclass.initComponent.call(this);

        this.store = new Ext.data.ArrayStore({
            fields: [
                { name: 'address', type: 'string' },
                { name: 'client', type: 'string' },
                { name: 'sent', type: 'int' },
                { name: 'received', type: 'int' },
                { name: 'ratio', type: 'float' },
                { name: 'torrents', type: 'int' },
                { name: 'cross_seeds', type: 'int' },
                { name: 'last_seen', type: 'float' },
            ],
        });

        // The settings live here rather than in Preferences because this is
        // where somebody finds out the ledger exists, and an empty window with
        // no way to turn it on from where you are looking is a dead end.
        this.enabled = new Ext.form.Checkbox({
            boxLabel: _('Keep a history of peers'),
            hideLabel: true,
            listeners: { check: { fn: this.onSwitched, scope: this } },
        });
        this.ttl = new Ext.ux.form.SpinnerField({
            hideLabel: true,
            width: 60,
            decimalPrecision: 0,
            minValue: 1,
            maxValue: 3650,
            incrementValue: 1,
            value: 30,
        });
        this.ttl.on('spin', this.onSwitched, this);
        this.ttl.on('blur', this.onSwitched, this);

        this.grid = this.add({
            xtype: 'grid',
            store: this.store,
            border: false,
            // On the grid rather than as a second item of this window: the
            // layout is `fit`, which draws one thing.
            tbar: [
                this.enabled,
                '  ',
                '-',
                '  ',
                _('Forget after (days):'),
                ' ',
                this.ttl,
                '->',
                {
                    text: _('Refresh'),
                    iconCls: 'icon-ok',
                    handler: this.load,
                    scope: this,
                },
            ],
            viewConfig: {
                emptyText: _('No peers recorded yet.'),
                deferEmptyText: false,
            },
            columns: [
                {
                    header: _('Address'),
                    dataIndex: 'address',
                    width: 140,
                    renderer: Ext.util.Format.htmlEncode,
                },
                {
                    header: _('Client'),
                    dataIndex: 'client',
                    width: 150,
                    renderer: Ext.util.Format.htmlEncode,
                },
                {
                    header: _('Took'),
                    dataIndex: 'sent',
                    width: 90,
                    renderer: fsize,
                },
                {
                    header: _('Gave'),
                    dataIndex: 'received',
                    width: 90,
                    renderer: fsize,
                },
                {
                    header: _('Gave back'),
                    dataIndex: 'ratio',
                    width: 80,
                    renderer: this.renderRatio,
                },
                {
                    header: _('Torrents'),
                    dataIndex: 'torrents',
                    width: 70,
                },
                {
                    header: _('Cross-seeds'),
                    dataIndex: 'cross_seeds',
                    width: 95,
                },
                {
                    header: _('Last seen'),
                    dataIndex: 'last_seen',
                    width: 130,
                    renderer: this.renderLastSeen,
                },
            ],
        });

        this.addButton(_('Close'), this.onClose, this);

        this.on('show', this.onShown, this);
        this.on('hide', this.stopPolling, this);
        this.on('destroy', this.stopPolling, this);
    },

    onShown: function () {
        this.loadSettings();
        this.load();
        this.stopPolling();
        // Only while it is open: a window nobody is looking at has no business
        // asking the daemon anything.
        this.timer = window.setInterval(
            this.load.createDelegate(this),
            this.interval * 1000
        );
    },

    stopPolling: function () {
        if (!this.timer) return;
        window.clearInterval(this.timer);
        this.timer = null;
    },

    onClose: function () {
        this.hide();
    },

    /**
     * Reads whether the ledger is on, and how long it remembers.
     */
    loadSettings: function () {
        deluge.client.core.get_config({
            success: function (config) {
                var settings = (config && config['peers']) || {};
                // Set without firing the write below back at the daemon.
                this.loading = true;
                this.enabled.setValue(settings['enabled'] === true);
                this.ttl.setValue(Deluge.number(settings['ttl_days'], 30));
                this.loading = false;
            },
            failure: function () {
                this.loading = false;
            },
            scope: this,
        });
    },

    onSwitched: function () {
        if (this.loading) return;
        deluge.client.core.set_config({
            peers: {
                enabled: this.enabled.getValue() === true,
                ttl_days: Deluge.number(this.ttl.getValue(), 30),
            },
        });
    },

    load: function () {
        deluge.client.redeluge.get_peers(500, {
            success: function (peers) {
                if (!this.isVisible()) return;
                var rows = [];
                Ext.each(peers || [], function (peer) {
                    var sent = Number(peer['sent']) || 0;
                    var received = Number(peer['received']) || 0;
                    rows.push([
                        peer['address'],
                        peer['client'] || '',
                        sent,
                        received,
                        // -1 for "it never took anything", which sorts below
                        // every real ratio and draws as a dash.
                        sent > 0 ? received / sent : -1,
                        peer['torrents'],
                        peer['cross_seeds'],
                        peer['last_seen'],
                    ]);
                });
                this.store.loadData(rows);
            },
            failure: function () {
                if (!this.isVisible()) return;
                this.store.removeAll();
            },
            scope: this,
        });
    },

    /**
     * What it gave back for what it took.
     *
     * A dash when it took nothing: "asked for nothing" and "asked and gave
     * nothing" are different facts, and only the second is worth reading.
     */
    renderRatio: function (value) {
        if (value < 0) return '&ndash;';
        return new Number(value).toFixed(3);
    },

    renderLastSeen: function (value) {
        var seconds = Math.round(new Date().getTime() / 1000 - Number(value));
        if (!value) return '';
        if (seconds < 60) return _('just now');
        if (seconds < 86400) return String.format(_('{0} ago'), ftime(seconds));
        return fdate(value);
    },
});
