/**
 * Deluge.preferences.OtherPage.js
 *
 * Copyright (c) Damien Churchill 2009-2010 <damoxc@gmail.com>
 *
 * This file is part of Deluge and is licensed under GNU General Public License 3.0, or later, with
 * the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */
Ext.namespace('Deluge.preferences');

/**
 * @class Deluge.preferences.Other
 * @extends Ext.form.FormPanel
 */
Deluge.preferences.Other = Ext.extend(Ext.form.FormPanel, {
    constructor: function (config) {
        config = Ext.apply(
            {
                border: false,
                title: _('Other'),
                header: false,
                layout: 'form',
            },
            config
        );
        Deluge.preferences.Other.superclass.constructor.call(this, config);
    },

    initComponent: function () {
        Deluge.preferences.Other.superclass.initComponent.call(this);

        var optMan = deluge.preferences.getOptionsManager();

        // What was here: a release check and an anonymous-statistics
        // upload. Neither has anything behind it. redeluge has no update
        // service to ask and sends nothing anywhere, so both were a
        // preference that could be set and could not mean anything.
        var fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('GeoIP Database'),
            autoHeight: true,
            labelWidth: 80,
            defaultType: 'textfield',
        });
        fieldset.add({
            xtype: 'label',
            text: _(
                'Peers show the flag of their country, which needs a database. Deluge pointed at a system file in a format retired in 2019, so there has been nothing to read for years.'
            ),
            style: 'display: block; margin-bottom: 6px; color: #666;',
        });
        optMan.bind(
            'geoip_db_location',
            fieldset.add({
                name: 'geoip_db_location',
                fieldLabel: _('Your own file:'),
                labelSeparator: '',
                width: 200,
            })
        );

        // The downloader. Its own dictionary under `countrydb`, so its own
        // read and its own Apply, like the other feature settings.
        this.db = {};
        this.db.enabled = fieldset.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Or download one and keep it up to date'),
            handler: this.onDatabaseToggled,
            scope: this,
        });
        this.db.url = fieldset.add({
            xtype: 'textfield',
            fieldLabel: _('From:'),
            labelSeparator: '',
            width: 260,
        });
        fieldset.add({
            xtype: 'label',
            text: _(
                'The default is DB-IP\u2019s free country database, which is CC BY 4.0 and needs no account. {YYYY-MM} in the address is filled in: the file is published monthly.'
            ),
            style: 'display: block; margin: 2px 0 6px 0; color: #666;',
        });
        this.db.status = fieldset.add({
            xtype: 'label',
            text: _('Not downloaded.'),
            style: 'display: block; color: #666;',
        });

        this.setDatabaseEnabled(false);
        this.on('show', this.onPageShow, this);
    },

    onPageShow: function () {
        if (this.dbLoaded) return;
        this.dbLoaded = true;
        deluge.client.core.get_config_value('countrydb', {
            success: function (settings) {
                settings = settings || {};
                this.db.enabled.setValue(settings['enabled'] === true);
                this.db.url.setValue(Ext.value(settings['url'], ''));

                var last = Number(settings['last_update'] || 0);
                this.db.status.setText(
                    last > 0
                        ? String.format(
                              _('Last downloaded {0}.'),
                              new Date(last * 1000).toLocaleString()
                          )
                        : _('Not downloaded.')
                );
                this.onDatabaseToggled();
            },
            failure: function () {
                this.onDatabaseToggled();
            },
            scope: this,
        });
    },

    onDatabaseToggled: function () {
        this.setDatabaseEnabled(this.db.enabled.getValue() === true);
    },

    setDatabaseEnabled: function (on) {
        this.db.url.setDisabled(!on);
    },

    onApply: function () {
        // Nothing read means nothing of this page's to write.
        if (!this.dbLoaded) return;
        deluge.client.core.set_config({
            countrydb: {
                enabled: this.db.enabled.getValue() === true,
                url: this.db.url.getValue() || '',
            },
        });
    },
});
