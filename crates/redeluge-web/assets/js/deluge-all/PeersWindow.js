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
    height: 560,
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
            // What the daemon answers in, so the first thing on screen is the
            // biggest taker; the header clicks take it from there.
            sortInfo: { field: 'sent', direction: 'DESC' },
            fields: [
                { name: 'address', type: 'string' },
                { name: 'client', type: 'string' },
                { name: 'sent', type: 'int' },
                { name: 'received', type: 'int' },
                { name: 'ratio', type: 'float' },
                { name: 'torrents', type: 'int' },
                // The torrents themselves, for the panel below the grid. No
                // type: it is a list of objects, not a value to sort on.
                { name: 'shared' },
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

        this.detail = new Ext.Panel({
            region: 'south',
            height: 120,
            split: true,
            minHeight: 60,
            border: false,
            autoScroll: true,
            bodyStyle: 'padding: 6px 8px;',
            html: this.emptyDetail(),
        });

        this.grid = new Ext.grid.GridPanel({
            region: 'center',
            store: this.store,
            border: false,
            sm: new Ext.grid.RowSelectionModel({ singleSelect: true }),
            // On the grid rather than as a second item of this window: the
            // layout is `fit`, which draws one thing.
            tbar: [
                {
                    // Sent to the daemon rather than applied here: the window
                    // holds the five hundred biggest takers, and the peer
                    // somebody is looking for is usually not one of those.
                    xtype: 'textfield',
                    width: 150,
                    emptyText: _('Address or client'),
                    enableKeyEvents: true,
                    listeners: {
                        keyup: {
                            fn: this.onSearchKey,
                            scope: this,
                            // A request per keystroke would be one per letter.
                            buffer: 400,
                        },
                    },
                    ref: '../../search',
                },
                ' ',
                '-',
                ' ',
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
                    // The circular arrow, which is what `update.png` is; the
                    // tick this used to wear meant nothing here.
                    iconCls: 'icon-update-tracker',
                    handler: this.load,
                    scope: this,
                },
            ],
            viewConfig: {
                emptyText: _('No peers recorded yet.'),
                deferEmptyText: false,
            },
            listeners: {
                // The empty text is fixed at render, and "no peers recorded
                // yet" is a lie once something has been typed in the box.
                viewready: { fn: this.rememberEmptyText, scope: this },
            },
            columns: [
                {
                    header: _('Address'),
                    sortable: true,
                    dataIndex: 'address',
                    width: 140,
                    renderer: Ext.util.Format.htmlEncode,
                },
                {
                    header: _('Client'),
                    sortable: true,
                    dataIndex: 'client',
                    width: 150,
                    renderer: Ext.util.Format.htmlEncode,
                },
                {
                    header: _('Took'),
                    sortable: true,
                    dataIndex: 'sent',
                    width: 90,
                    renderer: fsize,
                },
                {
                    header: _('Gave'),
                    sortable: true,
                    dataIndex: 'received',
                    width: 90,
                    renderer: fsize,
                },
                {
                    header: _('Gave back'),
                    sortable: true,
                    dataIndex: 'ratio',
                    width: 80,
                    renderer: this.renderRatio,
                },
                {
                    header: _('Torrents'),
                    sortable: true,
                    dataIndex: 'torrents',
                    width: 70,
                },
                {
                    header: _('Cross-seeds'),
                    sortable: true,
                    dataIndex: 'cross_seeds',
                    width: 95,
                },
                {
                    header: _('Last seen'),
                    sortable: true,
                    dataIndex: 'last_seen',
                    width: 130,
                    renderer: this.renderLastSeen,
                },
            ],
        });
        this.grid.getSelectionModel().on('rowselect', this.onPeerSelected, this);

        // Border layout rather than fit: the grid answers "who", the panel
        // under it answers "in which of my torrents", and the second question
        // is always asked immediately after the first.
        this.add(
            new Ext.Panel({
                layout: 'border',
                border: false,
                items: [this.grid, this.detail],
            })
        );

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

    rememberEmptyText: function () {
        this.emptyWithout = this.grid.getView().emptyText;
    },

    setEmptyText: function () {
        var view = this.grid.getView();
        if (!view || !this.emptyWithout) return;
        view.emptyText = this.query
            ? Ext.util.Format.htmlEncode(
                  String.format(_('No peer matches "{0}".'), this.query)
              )
            : this.emptyWithout;
    },

    onSearchKey: function (field) {
        var wanted = field.getValue() || '';
        if (wanted === this.query) return;
        this.query = wanted;
        this.load();
    },

    load: function () {
        deluge.client.redeluge.get_peers(500, this.query || '', {
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
                        (peer['torrents'] || []).length,
                        peer['torrents'] || [],
                        peer['cross_seeds'],
                        peer['last_seen'],
                    ]);
                });
                var chosen = this.grid.getSelectionModel().getSelected();
                this.setEmptyText();
                this.store.loadData(rows);
                // The reload drops the selection, and a detail panel about a
                // peer that is no longer highlighted is worse than none.
                var index = chosen
                    ? this.store.find('address', chosen.get('address'))
                    : -1;
                if (index > -1) {
                    this.grid.getSelectionModel().selectRow(index);
                } else if (this.detail.body) {
                    this.detail.body.update(this.emptyDetail());
                }
            },
            failure: function () {
                if (!this.isVisible()) return;
                this.store.removeAll();
                if (this.detail.body) this.detail.body.update(this.emptyDetail());
            },
            scope: this,
        });
    },

    emptyDetail: function () {
        return (
            '<span style="color: #888;">' +
            Ext.util.Format.htmlEncode(
                _('Pick a peer to see which of your torrents it was seen in.')
            ) +
            '</span>'
        );
    },

    /**
     * Which of your torrents this peer has been seen in.
     *
     * The count in the grid says how many and nothing else; this says which,
     * because that is the next thing anybody asks. A torrent removed since is
     * still listed, by its infohash: what it moved is no less true for the
     * torrent being gone.
     */
    onPeerSelected: function (model, index, record) {
        var shared = record.get('shared') || [];
        if (!shared.length) {
            this.detail.body.update(this.emptyDetail());
            return;
        }

        var rows = [];
        Ext.each(shared, function (entry) {
            var name = entry['name'] || entry['hash'] || '';
            var line = Ext.util.Format.htmlEncode(name);
            if (!entry['name']) {
                line =
                    '<span style="color: #888;">' +
                    line +
                    ' ' +
                    Ext.util.Format.htmlEncode(_('(removed)')) +
                    '</span>';
            }
            if (entry['cross_seed']) {
                line +=
                    ' <span style="color: #888;">' +
                    Ext.util.Format.htmlEncode(
                        _('- also on another of your torrents')
                    ) +
                    '</span>';
            }
            rows.push('<div>' + line + '</div>');
        });

        this.detail.body.update(
            '<div style="margin-bottom: 4px;"><b>' +
                Ext.util.Format.htmlEncode(record.get('address')) +
                '</b> ' +
                Ext.util.Format.htmlEncode(_('was seen in:')) +
                '</div>' +
                rows.join('')
        );
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
