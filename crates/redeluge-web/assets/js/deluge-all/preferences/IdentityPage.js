/**
 * Deluge.preferences.IdentityPage.js
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * What this client tells the swarm it is.
 *
 * Two strings make up an identity on the wire and they have to agree: the user
 * agent, which trackers see as an HTTP header and peers see in the extension
 * handshake, and the peer id, whose first bytes are a fingerprint like
 * `-qB4650-`. Faking one and not the other is worse than faking neither, so
 * every mode here sets both or neither, and the custom boxes can be filled
 * from a real client in one go rather than typed a half at a time.
 *
 * This was a checkbox in the proxy widget. It is not a proxy setting: it
 * changes what this client says about itself, and it does that the same way
 * whether a proxy is configured or not. Whatever is chosen here, the address
 * the packets come from does not change, and the page says so.
 *
 * Stored under the daemon's own `identity` key. The old `proxy.anonymous_mode`
 * is carried into it once, on the daemon's first start after the upgrade.
 */
Ext.namespace('Deluge.preferences');

/**
 * @class Deluge.preferences.Identity
 * @extends Ext.form.FormPanel
 */
Deluge.preferences.Identity = Ext.extend(Ext.form.FormPanel, {
    constructor: function (config) {
        config = Ext.apply(
            {
                border: false,
                title: _('Identity'),
                header: false,
                layout: 'form',
                autoScroll: true,
            },
            config
        );
        Deluge.preferences.Identity.superclass.constructor.call(this, config);
    },

    initComponent: function () {
        Deluge.preferences.Identity.superclass.initComponent.call(this);

        this.setting = false;
        this.modes = {};

        var fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('What the swarm is told'),
            autoHeight: true,
            labelWidth: 1,
            style: 'padding-top: 5px; margin-bottom: 0px;',
        });

        Ext.each(
            Deluge.preferences.Identity.MODES,
            function (mode) {
                this.modes[mode.key] = fieldset.add({
                    xtype: 'radio',
                    name: 'identity_mode',
                    inputValue: mode.key,
                    hideLabel: true,
                    boxLabel: mode.label,
                    style: mode.key === 'show' ? '' : 'margin-top: 6px',
                    listeners: { check: this.onModeCheck, scope: this },
                });
                var hint = fieldset.add({
                    xtype: 'label',
                    text: mode.hint,
                    style:
                        'display: block; margin: 1px 0 0 18px; color: #666;',
                });
                // Kept, because this one is filled in from the daemon once the
                // list of clients arrives.
                if (mode.key === 'rotate') this.rotateHint = hint;
            },
            this
        );

        // The custom half, indented under the mode that uses it and disabled
        // under every other one.
        var custom = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Custom identity'),
            autoHeight: true,
            labelWidth: 120,
            style: 'padding-top: 5px; margin-bottom: 0px;',
        });

        this.knownClient = custom.add({
            xtype: 'combo',
            fieldLabel: _('Copy from:'),
            labelSeparator: '',
            width: 220,
            mode: 'local',
            triggerAction: 'all',
            editable: false,
            emptyText: _('a client this daemon knows'),
            valueField: 'name',
            displayField: 'name',
            store: new Ext.data.JsonStore({
                fields: ['name', 'user_agent', 'peer_id'],
                data: [],
            }),
            listeners: { select: this.onKnownClient, scope: this },
        });

        this.userAgent = custom.add({
            xtype: 'textfield',
            fieldLabel: _('User agent:'),
            labelSeparator: '',
            width: 300,
        });
        this.userAgent.on('change', this.onFieldChange, this);

        this.peerId = custom.add({
            xtype: 'textfield',
            fieldLabel: _('Peer id prefix:'),
            labelSeparator: '',
            width: 140,
        });
        this.peerId.on('change', this.onFieldChange, this);

        custom.add({
            xtype: 'label',
            text: _(
                'The prefix is the first bytes of every peer id this daemon sends, conventionally eight characters like -qB4650-. Anything past twenty is cut off. Leave a box empty to keep this daemon’s real value for that half.'
            ),
            style: 'display: block; margin: 4px 0 0 0; color: #666;',
        });

        // The honest half, and the reason this page is not called Privacy.
        this.add({
            xtype: 'fieldset',
            border: false,
            title: _('What none of this does'),
            autoHeight: true,
            labelWidth: 1,
            style: 'padding-top: 5px; margin-bottom: 0px;',
            items: [
                {
                    xtype: 'label',
                    text: _(
                        'It does not hide your address. Every peer and tracker still sees the packets arrive from your IP, whichever name they are told. This page changes what the client says, not where it says it from — that is the proxy, on its own page.'
                    ),
                    style: 'display: block; color: #666;',
                },
            ],
        });

        deluge.preferences.getOptionsManager().bind('identity', this);
        this.loadClients();
    },

    /**
     * The clients `rotate` draws from, named by the daemon rather than listed
     * here, so the two cannot drift apart.
     */
    loadClients: function () {
        if (!deluge.client || !deluge.client.redeluge) return;
        deluge.client.redeluge.get_identity_clients({
            success: function (clients) {
                this.clients = clients || [];
                this.knownClient.getStore().loadData(this.clients);
                this.describeRotation();
            },
            failure: function () {
                // The daemon may not be connected yet. The mode still works;
                // only the list of names is missing.
                this.clients = [];
            },
            scope: this,
        });
    },

    /**
     * Names the clients under the rotate option, once they are known.
     */
    describeRotation: function () {
        if (!this.rotateHint || !this.clients || !this.clients.length) return;
        var names = this.clients.map(function (client) {
            return client.name;
        });
        this.rotateHint.setText(
            String.format(
                _(
                    'Drawn from: {0}. The peer id is drawn for each torrent as it is added and kept for that torrent’s life. The user agent is one string for the whole daemon, so it follows the most recent draw and older torrents will disagree with it — which a tracker comparing the two can see.'
                ),
                names.join(', ')
            )
        );
    },

    getValue: function () {
        return {
            mode: this.mode(),
            user_agent: this.userAgent.getValue() || '',
            peer_id: this.peerId.getValue() || '',
        };
    },

    setValue: function (value) {
        this.setting = true;
        value = value || {};

        var mode = value['mode'];
        if (!this.modes[mode]) mode = 'show';
        for (var key in this.modes) {
            this.modes[key].setValue(key === mode);
        }
        this.userAgent.setValue(value['user_agent'] || '');
        this.peerId.setValue(value['peer_id'] || '');

        this.setting = false;
        this.onModeChanged();
    },

    /**
     * Which radio is on. `show` when none is, which is what an empty or
     * unreadable setting has to mean: this page must never invent an identity.
     */
    mode: function () {
        for (var key in this.modes) {
            if (this.modes[key].getValue() === true) return key;
        }
        return 'show';
    },

    onModeCheck: function (radio, checked) {
        // A radio group fires twice per change, once for the box going off.
        if (!checked) return;
        this.onModeChanged();
        this.onFieldChange(radio, true, false);
    },

    /**
     * The custom boxes mean nothing under the other three modes.
     */
    onModeChanged: function () {
        var custom = this.mode() === 'custom';
        this.knownClient.setDisabled(!custom);
        this.userAgent.setDisabled(!custom);
        this.peerId.setDisabled(!custom);
    },

    /**
     * Fills both boxes at once, which is the point of offering the list: a
     * user agent from one client and a peer id from another is a combination
     * nobody else in the world sends.
     */
    onKnownClient: function (combo, record) {
        this.userAgent.setValue(record.get('user_agent'));
        this.peerId.setValue(record.get('peer_id'));
        this.onFieldChange(combo, record.get('name'), '');
    },

    onFieldChange: function (field, newValue, oldValue) {
        if (this.setting) return;
        var newValues = this.getValue();
        var oldValues = Ext.apply({}, newValues);
        this.fireEvent('change', this, newValues, oldValues);
    },
});

/**
 * The four modes, in the order they are offered.
 *
 * The hints are as plain as they can be made: this is a page where a wrong
 * belief about what a setting does is the whole risk.
 */
Deluge.preferences.Identity.MODES = [
    {
        key: 'show',
        label: _('Show what this client is'),
        hint: _('redeluge, and the libtorrent it is built on. The default.'),
    },
    {
        key: 'hide',
        label: _('Hide the client identity'),
        hint: _(
            'A generic agent to trackers, no version to peers. Private torrents are exempt: a private tracker checks which client it is talking to.'
        ),
    },
    {
        key: 'rotate',
        label: _('Look like a different common client for each torrent'),
        hint: _(
            'The peer id is drawn for each torrent added; the user agent is one string for the whole daemon and follows the most recent draw.'
        ),
    },
    {
        key: 'custom',
        label: _('Say exactly this'),
        hint: _('Both halves, set below.'),
    },
];
