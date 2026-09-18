/**
 * Deluge.preferences.NotificationsPage.js
 *
 * Copyright (c) 2026 the redeluge contributors
 *
 * This file is part of redeluge and is licensed under GNU General Public License 3.0, or later,
 * with the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */
Ext.namespace('Deluge.preferences');

/**
 * @class Deluge.preferences.Notifications
 * @extends Ext.Panel
 *
 * Where a message goes when a torrent finishes, arrives or breaks. The
 * `webhook` key of core.conf, one dictionary holding a list of destinations.
 *
 * An editable grid rather than a dialog per destination: every field is a
 * short string or a choice, and somebody with a phone topic and a Discord
 * channel wants to see both at once.
 */
Deluge.preferences.Notifications = Ext.extend(Ext.Panel, {
    border: false,
    title: _('Notifications'),
    header: false,
    layout: 'form',
    autoScroll: true,

    initComponent: function () {
        Deluge.preferences.Notifications.superclass.initComponent.call(this);

        var fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Notifications'),
            autoHeight: true,
            labelWidth: 140,
            style: 'padding-top: 5px; margin-bottom: 0px;',
        });

        this.enabled = fieldset.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Send a message when something happens to a torrent'),
        });
        // Not `onFinished`, `onError` or `onAdded`: `onAdded` is a method
        // every Ext.Component has, and a checkbox parked on that name replaces
        // it, so the framework calls the checkbox when this panel is added to
        // the window and the whole page never renders. The other two are
        // renamed for the same reason before they become one.
        this.finishedBox = fieldset.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('A download finished'),
            ctCls: 'x-deluge-indent-checkbox',
        });
        this.errorBox = fieldset.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('A torrent went into error'),
            ctCls: 'x-deluge-indent-checkbox',
        });
        this.addedBox = fieldset.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('A torrent was added'),
            ctCls: 'x-deluge-indent-checkbox',
        });

        this.store = new Ext.data.ArrayStore({
            fields: [
                { name: 'enabled', type: 'bool' },
                { name: 'kind', type: 'string' },
                { name: 'url', type: 'string' },
                { name: 'token', type: 'string' },
            ],
        });

        var services = new Ext.data.SimpleStore({
            fields: ['value', 'text'],
            data: [
                ['discord', _('Discord')],
                ['ntfy', _('ntfy')],
                ['gotify', _('Gotify')],
                ['webhook', _('Webhook')],
            ],
        });

        this.grid = this.add({
            xtype: 'editorgrid',
            store: this.store,
            height: 180,
            anchor: '100%',
            clicksToEdit: 1,
            style: 'margin: 8px 0;',
            selModel: new Ext.grid.RowSelectionModel({ singleSelect: true }),
            columns: [
                {
                    header: _('On'),
                    dataIndex: 'enabled',
                    width: 36,
                    renderer: this.renderTick,
                    editor: { xtype: 'checkbox' },
                },
                {
                    header: _('Service'),
                    dataIndex: 'kind',
                    width: 90,
                    editor: new Ext.form.ComboBox({
                        store: services,
                        displayField: 'text',
                        valueField: 'value',
                        mode: 'local',
                        triggerAction: 'all',
                        editable: false,
                    }),
                    renderer: function (value) {
                        var found = services.data.find(function (item) {
                            return item.data.value == value;
                        });
                        return found ? found.data.text : value;
                    },
                },
                {
                    header: _('URL'),
                    dataIndex: 'url',
                    width: 280,
                    editor: { xtype: 'textfield' },
                },
                {
                    header: _('Token'),
                    dataIndex: 'token',
                    width: 110,
                    editor: { xtype: 'textfield' },
                    // A token is a secret sitting on a shared screen, and
                    // nobody needs to read it back: they need to know whether
                    // there is one.
                    renderer: function (value) {
                        return value ? '••••••' : '';
                    },
                },
            ],
            bbar: [
                {
                    text: _('Add'),
                    iconCls: 'icon-add',
                    handler: this.onAddEndpoint,
                    scope: this,
                },
                {
                    text: _('Remove'),
                    iconCls: 'icon-remove',
                    handler: this.onRemoveEndpoint,
                    scope: this,
                },
                '->',
                {
                    text: _('Send test'),
                    iconCls: 'icon-ok',
                    handler: this.onSendTest,
                    scope: this,
                },
            ],
        });

        this.result = this.add({
            xtype: 'label',
            text: '',
            style: 'display: block; margin: 0 0 6px 0; opacity: 0.72;',
        });

        this.add({
            xtype: 'label',
            text: _(
                'Discord takes a channel webhook URL. ntfy takes the topic URL you would open in the app. Gotify takes the server URL and an application token. Webhook posts one JSON object to anything else.'
            ),
            style: 'display: block; margin: 0 0 8px 0; opacity: 0.72;',
        });

        this.on('show', this.onPageShow, this);
    },

    renderTick: function (value) {
        return value ? '&#10003;' : '';
    },

    // Not `onAdd` or `onRemove`: Ext.Container owns both of those names and
    // calls them itself whenever anything is added to the panel.
    onAddEndpoint: function () {
        this.store.add(
            new this.store.recordType({
                enabled: true,
                kind: 'discord',
                url: '',
                token: '',
            })
        );
    },

    onRemoveEndpoint: function () {
        var selected = this.grid.getSelectionModel().getSelected();
        if (selected) this.store.remove(selected);
    },

    /**
     * Sends one sample message, so a URL is checked before it matters.
     *
     * It writes the page first: testing what is on screen is the only useful
     * thing to test. The daemon clears the flag and writes back what happened,
     * which is what the line under the grid then shows.
     */
    onSendTest: function () {
        this.result.setText(_('Sending...'));
        this.apply(true);
        this.pollFor = 10;
        this.pollTest.defer(1500, this);
    },

    pollTest: function () {
        deluge.client.core.get_config_value('webhook', {
            success: function (settings) {
                settings = settings || {};
                if (settings['test'] === true && this.pollFor-- > 0) {
                    this.pollTest.defer(1500, this);
                    return;
                }
                this.result.setText(
                    settings['last_test'] || _('No answer from the daemon')
                );
            },
            failure: function () {
                this.result.setText(_('No answer from the daemon'));
            },
            scope: this,
        });
    },

    onPageShow: function () {
        if (this.loaded) return;
        this.loaded = true;
        deluge.client.core.get_config_value('webhook', {
            success: this.onGotConfig,
            scope: this,
        });
    },

    onGotConfig: function (settings) {
        settings = settings || {};
        this.enabled.setValue(settings['enabled'] === true);
        this.finishedBox.setValue(settings['on_finished'] !== false);
        this.errorBox.setValue(settings['on_error'] !== false);
        this.addedBox.setValue(settings['on_added'] === true);
        this.result.setText(settings['last_test'] || '');
        // Kept and written back rather than dropped: neither has a control
        // here, and set_config replaces the whole dictionary, so a value
        // somebody set through the API would vanish on the next Apply.
        this.timeout = Ext.value(settings['timeout'], 15);
        this.tryTimes = Ext.value(settings['try_times'], 3);

        var rows = (settings['endpoints'] || []).map(function (end) {
            return [
                end['enabled'] !== false,
                Ext.value(end['kind'], 'discord'),
                Ext.value(end['url'], ''),
                Ext.value(end['token'], ''),
            ];
        });
        this.store.loadData(rows);
    },

    onApply: function () {
        this.apply(false);
    },

    apply: function (test) {
        // Preferences applies every page, including ones nobody opened. Until
        // this one has read the stored settings its grid is empty, and writing
        // that back would delete every destination.
        if (!this.loaded) return;

        var endpoints = [];
        this.store.each(function (record) {
            var url = (record.get('url') || '').trim();
            // A row with no URL is a row somebody added and did not fill in.
            if (!url) return;
            endpoints.push({
                enabled: record.get('enabled') === true,
                kind: record.get('kind') || 'discord',
                url: url,
                token: record.get('token') || '',
            });
        });

        deluge.client.core.set_config({
            webhook: {
                enabled: this.enabled.getValue() === true,
                on_finished: this.finishedBox.getValue() === true,
                on_error: this.errorBox.getValue() === true,
                on_added: this.addedBox.getValue() === true,
                endpoints: endpoints,
                timeout: Ext.value(this.timeout, 15),
                try_times: Ext.value(this.tryTimes, 3),
                test: test === true,
            },
        });
    },

    onOk: function () {
        this.onApply();
    },
});
